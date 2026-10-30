use slint::{Color, ModelRc, VecModel};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use ikaros_adb::AdbClient;
use ikaros_domain::{AppFilterType, LogLevel};
use ikaros_usecase::{AppUseCases, DeviceUseCases, FileUseCases, LogcatUseCases};

mod app_config;
#[cfg(test)]
mod device_ui_tests;
mod file_ui;
mod log_filter;
mod log_search_ui;
mod log_selection_ui;
#[cfg(target_os = "macos")]
mod macos_menu;
#[cfg(target_os = "macos")]
mod macos_window;
mod mvi;
mod performance_ui;
mod scrcpy;
mod scrcpy_parameters;
mod scrcpy_tags;
mod scrcpy_ui;
mod ui_bindings;
use mvi::{AppIntent, AppStore, AppViewState};

slint::include_modules!();

/// 确保 macOS GUI 环境下的 PATH 包含常见的 adb 及 Android SDK 工具目录
fn ensure_environment_path() {
    let mut current_path = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let candidate_dirs = [
        format!("{}/Library/Android/sdk/platform-tools", home),
        format!("{}/.cargo/bin", home),
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/bin".to_string(),
        "/bin".to_string(),
    ];
    for dir in candidate_dirs {
        if std::path::Path::new(&dir).exists() && !current_path.contains(&dir) {
            current_path = format!("{}:{}", dir, current_path);
        }
    }
    std::env::set_var("PATH", current_path);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 0. 环境变量预处理，保证 GUI 点击启动时可顺利定位 adb
    ensure_environment_path();

    // 1. 初始化终端日志追踪
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
    info!("启动 Ikaros Android 管理平台 (Clean Architecture + MVI)...");

    // 2. 组装整洁架构依赖图 (Clean Architecture Dependency Injection)
    let adb_repo = Arc::new(AdbClient::default());
    let device_usecases = Arc::new(DeviceUseCases::new(adb_repo.clone()));
    let file_usecases = Arc::new(FileUseCases::new(adb_repo.clone()));
    let app_usecases = Arc::new(AppUseCases::new(adb_repo.clone()));
    let logcat_usecases = Arc::new(LogcatUseCases::new(adb_repo.clone()));

    // 3. 构建 MVI 意图事件总线
    let (intent_tx, intent_rx) = mpsc::unbounded_channel::<AppIntent>();
    let store = AppStore::new(
        device_usecases.clone(),
        file_usecases.clone(),
        app_usecases.clone(),
        logcat_usecases.clone(),
        intent_tx.clone(),
        mvi::log_recording_directory(),
        Arc::new(ikaros_usecase::PerformanceUseCases::new(adb_repo.clone())),
    );

    // 4. 初始化 Slint UI 窗口
    #[cfg(target_os = "macos")]
    let _native_menu = macos_menu::init()?;
    let app = AppWindow::new()?;
    #[cfg(target_os = "macos")]
    macos_window::bind(app.window());
    let app_weak = app.as_weak();
    // 启动时恢复持久化的应用配置（外部 scrcpy 本体路径）
    {
        let config = app_config::load();
        if let Some(path) = config.scrcpy_path {
            app.global::<App>().set_scrcpy_path(path.into());
        }
    }
    performance_ui::bind(&app, intent_tx.clone());
    file_ui::bind(&app, intent_tx.clone());
    log_search_ui::bind(&app);
    log_selection_ui::bind(&app);
    scrcpy_ui::bind(&app);

    // 5. 绑定 Slint 回调到 MVI Intent 派发通道（实现在 ui_bindings.rs）
    ui_bindings::bind_all(&app, intent_tx.clone());

    // 6. 启动后台单向数据流 (MVI Loop)
    let _state_timer = start_state_loop(store, intent_rx, app_weak.clone());

    // 7. 启动设备热插拔自动监听 (host:track-devices)；adb server 重启导致流断开后自动重连
    {
        let usecases = device_usecases.clone();
        let tx = intent_tx.clone();
        tokio::spawn(async move {
            let mut announce_disconnected = false;
            loop {
                let (device_stream_tx, mut device_stream_rx) = mpsc::unbounded_channel();
                let uc = usecases.clone();
                tokio::spawn(async move {
                    let _ = uc.track_devices(device_stream_tx).await;
                });

                while let Some(devices) = device_stream_rx.recv().await {
                    if announce_disconnected {
                        announce_disconnected = false;
                        let _ = tx.send(AppIntent::SetStatusMessage {
                            message: "设备监听已恢复".into(),
                            is_busy: false,
                        });
                    }
                    let _ = tx.send(AppIntent::DevicesUpdated(devices));
                }

                if !announce_disconnected {
                    announce_disconnected = true;
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: "ADB 设备监听已断开，正在自动重连...".into(),
                        is_busy: false,
                    });
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    // 8. 触发初始加载
    let _ = intent_tx.send(AppIntent::RefreshDevices);

    // 9. 启动 Slint 主事件循环
    app.run()?;

    Ok(())
}

/// Keep only the latest snapshot; log bursts must never queue work on the UI thread.
fn start_state_loop(
    mut store: AppStore,
    mut intents: mpsc::UnboundedReceiver<AppIntent>,
    ui: slint::Weak<AppWindow>,
) -> slint::Timer {
    let (snapshots, mut latest) =
        watch::channel((0_u64, 0_u64, 0_u64, 0_u64, 0_u64, store.state.clone()));
    tokio::spawn(async move {
        let mut revision = 0_u64;
        let mut log_revision = 0_u64;
        let mut performance_revision = 0_u64;
        let mut shell_revision = 0_u64;
        let mut transfer_revision = 0_u64;
        while let Some(intent) = intents.recv().await {
            if matches!(&intent, AppIntent::PerformanceSampled { .. }) {
                performance_revision = performance_revision.wrapping_add(1);
            } else if matches!(
                &intent,
                AppIntent::ShellOutputReceived(_) | AppIntent::ShellCommandFinished { .. }
            ) {
                shell_revision = shell_revision.wrapping_add(1);
            } else if matches!(&intent, AppIntent::TransferProgressUpdated(_)) {
                transfer_revision = transfer_revision.wrapping_add(1);
            } else if matches!(&intent, AppIntent::LogEntriesReceived { .. }) {
                log_revision = log_revision.wrapping_add(1);
            } else {
                revision = revision.wrapping_add(1);
            }
            store.dispatch(intent).await;
            snapshots.send_replace((
                revision,
                log_revision,
                performance_revision,
                shell_revision,
                transfer_revision,
                store.state.clone(),
            ));
        }
    });

    let timer = slint::Timer::default();
    let mut applied_revision = None;
    let mut applied_log_revision = None;
    let mut applied_performance_revision = None;
    let mut applied_shell_revision = None;
    let mut applied_transfer_revision = None;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            if !latest.has_changed().unwrap_or(false) {
                return;
            }
            let snapshot = latest.borrow_and_update();
            let (main_rev, log_rev, perf_rev, shell_rev, transfer_rev, state) = &*snapshot;
            let Some(ui) = ui.upgrade() else { return };
            if applied_revision != Some(*main_rev) {
                apply_state_to_ui(&ui, state);
                applied_revision = Some(*main_rev);
                applied_log_revision = Some(*log_rev);
                applied_performance_revision = Some(*perf_rev);
                applied_shell_revision = Some(*shell_rev);
                applied_transfer_revision = Some(*transfer_rev);
            } else if state.active_tab == 4 && applied_log_revision != Some(*log_rev) {
                apply_logcat_to_ui(&ui, state);
                applied_log_revision = Some(*log_rev);
            } else if state.active_tab == 6 && applied_performance_revision != Some(*perf_rev) {
                performance_ui::apply(&ui, state);
                applied_performance_revision = Some(*perf_rev);
            } else if state.active_tab == 5 && applied_shell_revision != Some(*shell_rev) {
                // 终端高频输出走独立通道，只刷新输出文本，避免整页重建
                ui.global::<App>()
                    .set_shell_output(state.shell_output.clone().into());
                ui.global::<App>().set_is_shell_busy(state.is_shell_busy);
                applied_shell_revision = Some(*shell_rev);
            } else if applied_transfer_revision != Some(*transfer_rev) {
                // 传输进度走独立通道，只刷两个属性，避免高频整页重建
                ui.global::<App>()
                    .set_file_transfer_percent(state.file_transfer_percent);
                ui.global::<App>()
                    .set_file_status_text(state.file_transfer_message.clone().into());
                applied_transfer_revision = Some(*transfer_rev);
            }
        },
    );
    timer
}

fn apply_logcat_to_ui(ui: &AppWindow, state: &AppViewState) {
    ui.global::<App>()
        .set_is_logcat_active(state.is_logcat_streaming);
    ui.global::<App>()
        .set_is_log_recording_enabled(state.is_log_recording_enabled);
    ui.global::<App>()
        .set_is_log_recording_busy(state.is_log_recording_busy);
    ui.global::<App>()
        .set_log_recording_message(state.log_recording_message.clone().into());
    ui.global::<App>()
        .set_log_recording_has_error(state.log_recording_has_error);
    ui.global::<App>()
        .set_log_recording_directory(state.log_recording_directory.clone().into());
    ui.global::<App>()
        .set_log_search_query(state.log_search_query.clone().into());
    let query = mvi::log_query::LogQuery::parse(&state.log_search_query);
    ui.global::<App>()
        .set_log_show_package(!query.has_positive_filter(log_filter::Field::Package));
    ui.global::<App>()
        .set_log_show_tag(!query.has_positive_filter(log_filter::Field::Tag));
    let level = match state.log_level_filter {
        None => 0,
        Some(LogLevel::Verbose) => 1,
        Some(LogLevel::Debug) => 2,
        Some(LogLevel::Info) => 3,
        Some(LogLevel::Warn) => 4,
        Some(LogLevel::Error) | Some(LogLevel::Fatal) => 5,
    };
    ui.global::<App>().set_log_level_selected(level);
    let items: Vec<LogItemData> = state
        .display_logs
        .iter()
        .map(|log| {
            let color = match log.level {
                LogLevel::Fatal | LogLevel::Error => Color::from_rgb_u8(255, 85, 85),
                LogLevel::Warn => Color::from_rgb_u8(255, 184, 108),
                LogLevel::Info => Color::from_rgb_u8(80, 250, 123),
                LogLevel::Debug => Color::from_rgb_u8(248, 248, 242),
                LogLevel::Verbose => Color::from_rgb_u8(98, 114, 164),
            };
            LogItemData {
                timestamp: log.timestamp.clone().into(),
                level: log.level.as_str().into(),
                tag: log.tag.clone().into(),
                package_name: log
                    .package_name
                    .clone()
                    .unwrap_or_else(|| {
                        log.process_name
                            .as_ref()
                            .map(|name| format!("— ({name})"))
                            .unwrap_or_else(|| "—".into())
                    })
                    .into(),
                package_filter: log.package_name.clone().unwrap_or_default().into(),
                message: log.message.clone().into(),
                level_color: color,
            }
        })
        .collect();
    ui.global::<App>()
        .invoke_update_log_entries(ModelRc::from(std::rc::Rc::new(VecModel::from(items))));
}

/// 将 MVI State 投影更新到 Slint UI 属性（公共状态 + 当前激活页；其余页在切回时由 SwitchTab 触发投影）
fn apply_state_to_ui(ui: &AppWindow, state: &AppViewState) {
    apply_common_state(ui, state);
    match state.active_tab {
        2 => file_ui::apply(ui, state),
        3 => apply_packages_to_ui(ui, state),
        4 => apply_logcat_to_ui(ui, state),
        6 => {
            performance_ui::apply_choices(ui, state);
            performance_ui::apply(ui, state);
        }
        _ => {}
    }
}

/// 跨页共享的投影：设备下拉、全局状态、终端输出与选中设备的硬件详情
fn apply_common_state(ui: &AppWindow, state: &AppViewState) {
    // 1. 更新设备下拉列表
    let devices: Vec<DeviceItemData> = state
        .devices
        .iter()
        .map(|d| {
            let name = d.model.as_deref().unwrap_or(&d.serial);
            DeviceItemData {
                label: format!("{} ({})", name, d.serial).into(),
                serial: d.serial.clone().into(),
            }
        })
        .collect();
    let model = std::rc::Rc::new(VecModel::from(devices));
    ui.global::<App>().set_device_list(ModelRc::from(model));

    ui.global::<App>()
        .set_selected_device_index(state.selected_device_idx as i32);
    ui.global::<App>()
        .set_current_serial(state.current_serial().into());
    ui.global::<App>().set_is_busy(state.is_busy);
    ui.global::<App>()
        .set_status_message(state.status_message.clone().into());
    ui.global::<App>()
        .set_shell_output(state.shell_output.clone().into());
    ui.global::<App>().set_is_shell_busy(state.is_shell_busy);
    ui.global::<App>().set_active_tab(state.active_tab);
    ui.global::<App>().set_is_mirroring(state.is_mirroring());

    // 2. 更新设备硬件详情（概览与无线页共享）
    if let Some(detail) = state
        .selected_device_detail
        .as_ref()
        .filter(|detail| detail.serial == state.current_serial())
    {
        ui.global::<App>()
            .set_device_model(detail.model.clone().into());
        ui.global::<App>()
            .set_device_brand(detail.brand.clone().into());
        ui.global::<App>()
            .set_device_android_version(detail.android_version.clone().into());
        ui.global::<App>()
            .set_device_sdk(detail.sdk_api_level.to_string().into());
        ui.global::<App>()
            .set_device_resolution(detail.screen_resolution.clone().into());
        ui.global::<App>()
            .set_device_abi(detail.cpu_abi.clone().into());
        let battery_str = detail
            .battery_level
            .map(|l| format!("{}%", l))
            .unwrap_or_else(|| "未知".to_string());
        ui.global::<App>().set_device_battery(battery_str.into());
        let imei_str = detail.imei.as_deref().unwrap_or("未知");
        ui.global::<App>().set_device_imei(imei_str.into());
        ui.global::<App>().set_device_wifi_ip(
            detail
                .wifi_ip
                .as_deref()
                .unwrap_or("未连接或无法获取")
                .into(),
        );
        ui.global::<App>()
            .set_device_wifi_mac(detail.wifi_mac.as_deref().unwrap_or("无法获取").into());
        ui.global::<App>()
            .set_device_bluetooth_mac(detail.bluetooth_mac.as_deref().unwrap_or("无法获取").into());
    } else {
        ui.global::<App>().set_device_model("暂无选中设备".into());
        ui.global::<App>().set_device_brand("-".into());
        ui.global::<App>().set_device_android_version("-".into());
        ui.global::<App>().set_device_sdk("-".into());
        ui.global::<App>().set_device_resolution("-".into());
        ui.global::<App>().set_device_abi("-".into());
        ui.global::<App>().set_device_battery("-".into());
        ui.global::<App>().set_device_imei("-".into());
        ui.global::<App>().set_device_wifi_ip("-".into());
        ui.global::<App>().set_device_wifi_mac("-".into());
        ui.global::<App>().set_device_bluetooth_mac("-".into());
    }
}

/// 应用包列表的投影（含搜索过滤与图标加载）
fn apply_packages_to_ui(ui: &AppWindow, state: &AppViewState) {
    let filter_idx = match state.app_filter {
        AppFilterType::All => 0,
        AppFilterType::System => 1,
        AppFilterType::ThirdParty => 2,
    };
    ui.global::<App>().set_app_filter_tab(filter_idx);
    ui.global::<App>()
        .set_app_search_query(state.app_search_query.clone().into());

    let app_query = state.app_search_query.trim().to_lowercase();
    let app_items_vec: Vec<AppItemData> = state
        .app_list
        .iter()
        .filter(|app| {
            app_query.is_empty()
                || app.app_name.to_lowercase().contains(&app_query)
                || app.package_name.to_lowercase().contains(&app_query)
        })
        .map(|app| {
            let icon = app
                .icon_path
                .as_deref()
                .and_then(|path| slint::Image::load_from_path(path).ok());
            AppItemData {
                app_name: app.app_name.clone().into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
                package_name: app.package_name.clone().into(),
                version_name: if app.version_name.is_empty() {
                    "未知".into()
                } else {
                    app.version_name.clone().into()
                },
                version_code: app.version_code.clone().into(),
                is_system: app.is_system,
                apk_path: app.apk_path.clone().into(),
            }
        })
        .collect();
    let app_status = if app_query.is_empty() {
        state.app_status_message.clone()
    } else {
        format!(
            "匹配 {} / {} 个应用",
            app_items_vec.len(),
            state.app_list.len()
        )
    };
    ui.global::<App>().set_app_status_text(app_status.into());
    let app_model = std::rc::Rc::new(VecModel::from(app_items_vec));
    ui.global::<App>().set_app_items(ModelRc::from(app_model));
}
