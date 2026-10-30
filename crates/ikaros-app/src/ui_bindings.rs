use crate::{mvi::AppIntent, scrcpy, scrcpy_ui, App, AppWindow};
use ikaros_domain::{AppFilterType, LogLevel};
use slint::{ComponentHandle, ModelRc, SharedString};
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedSender;

/// 将所有 Slint 回调绑定到 MVI Intent 派发通道（按功能区拆分，main.rs 只负责装配与状态投影）
pub fn bind_all(app: &AppWindow, tx: UnboundedSender<AppIntent>) {
    let app_weak = app.as_weak();
    persist_scrcpy_path(app);
    // 启动时载入无线连接历史
    let startup_history: Vec<SharedString> = crate::app_config::load()
        .wireless_history
        .into_iter()
        .map(Into::into)
        .collect();
    app.global::<App>()
        .set_wireless_history(ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
            startup_history,
        ))));
    // 终端命令历史（仅保存在本会话内存中）
    let shell_history: std::rc::Rc<std::cell::RefCell<Vec<SharedString>>> = Default::default();
    let shell_cursor: std::rc::Rc<std::cell::Cell<usize>> = Default::default();

    // 5. 绑定 Slint 回调到 MVI Intent 派发通道
    {
        let tx = tx.clone();
        app.global::<App>().on_refresh_devices(move || {
            let _ = tx.send(AppIntent::RefreshDevices);
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_select_device(move |index| {
            let _ = tx.send(AppIntent::SelectDevice(index as usize));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_switch_tab(move |tab| {
            let _ = tx.send(AppIntent::SwitchTab(tab));
        });
    }

    {
        let tx = tx.clone();
        let shell_history = shell_history.clone();
        let shell_cursor = shell_cursor.clone();
        app.global::<App>().on_execute_shell(move |cmd| {
            // 记录命令历史供上下键翻阅
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                shell_history.borrow_mut().push(cmd.into());
                shell_cursor.set(shell_history.borrow().len());
            }
            let _ = tx.send(AppIntent::ExecuteShell(cmd.to_string()));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_interrupt_shell(move || {
            let _ = tx.send(AppIntent::InterruptShell);
        });
    }

    {
        let ui_handle = app_weak.clone();
        let shell_history = shell_history.clone();
        let shell_cursor = shell_cursor.clone();
        app.global::<App>().on_shell_history_up(move || {
            let Some(ui) = ui_handle.upgrade() else {
                return;
            };
            let items = shell_history.borrow();
            if items.is_empty() {
                return;
            }
            let index = shell_cursor.get().min(items.len()).saturating_sub(1);
            shell_cursor.set(index);
            ui.global::<App>()
                .set_shell_input_text(items[index].clone());
        });
    }
    {
        let ui_handle = app_weak.clone();
        let shell_history = shell_history.clone();
        let shell_cursor = shell_cursor.clone();
        app.global::<App>().on_shell_history_down(move || {
            let Some(ui) = ui_handle.upgrade() else {
                return;
            };
            let index = (shell_cursor.get() + 1).min(shell_history.borrow().len());
            shell_cursor.set(index);
            let text = shell_history
                .borrow()
                .get(index)
                .cloned()
                .unwrap_or_default();
            ui.global::<App>().set_shell_input_text(text);
        });
    }

    // 投屏相关交互: 一键启动时从 UI 读取可视化参数与自定义参数
    {
        let tx = tx.clone();
        let ui_handle = app_weak.clone();
        app.global::<App>().on_start_mirroring(move || {
            if let Some(ui) = ui_handle.upgrade() {
                let options = scrcpy::ScrcpyLaunchOptions {
                    binary_path: ui.global::<App>().get_scrcpy_path().to_string(),
                    max_size: scrcpy::numeric_preset(
                        &scrcpy::MAX_SIZE_PRESETS,
                        ui.global::<App>().get_scrcpy_max_size() as usize,
                    ),
                    max_fps: scrcpy::numeric_preset(
                        &scrcpy::MAX_FPS_PRESETS,
                        ui.global::<App>().get_scrcpy_max_fps() as usize,
                    ),
                    bit_rate: scrcpy::choice_preset(
                        &scrcpy::BIT_RATE_PRESETS,
                        ui.global::<App>().get_scrcpy_bit_rate() as usize,
                    ),
                    video_codec: scrcpy::choice_preset(
                        &scrcpy::VIDEO_CODECS,
                        ui.global::<App>().get_scrcpy_codec() as usize,
                    ),
                    audio: ui.global::<App>().get_scrcpy_audio(),
                    stay_awake: ui.global::<App>().get_scrcpy_stay_awake(),
                    extra_args: scrcpy::parse_extra_args(
                        &ui.global::<App>().get_scrcpy_extra_args(),
                    ),
                };
                scrcpy_ui::save_extra_args(ui.global::<App>().get_scrcpy_extra_args().to_string());
                let _ = tx.send(AppIntent::StartMirroring(options));
            }
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_stop_mirroring(move || {
            let _ = tx.send(AppIntent::StopMirroring);
        });
    }

    // 投屏本体路径: 打开系统原生文件夹选择器 (Finder) 回填路径框
    {
        let ui_handle = app_weak.clone();
        app.global::<App>().on_browse_scrcpy_path(move || {
            let Some(ui) = ui_handle.upgrade() else {
                return;
            };

            // 从当前已填路径出发定位起始目录, 减少重复导航
            let current = PathBuf::from(ui.global::<App>().get_scrcpy_path().to_string());
            let mut dialog = rfd::FileDialog::new().set_title("选择 scrcpy 所在文件夹");
            let start_dir = if current.is_dir() {
                Some(current)
            } else {
                current
                    .parent()
                    .map(|p| p.to_path_buf())
                    .filter(|p| p.is_dir())
            };
            if let Some(dir) = start_dir {
                dialog = dialog.set_directory(dir);
            }

            if let Some(folder) = dialog.pick_folder() {
                ui.global::<App>()
                    .set_scrcpy_path(folder.to_string_lossy().to_string().into());
            }
        });
    }

    // 应用管理器交互
    {
        let tx = tx.clone();
        app.global::<App>().on_search_apps(move |query| {
            let _ = tx.send(AppIntent::SetAppSearchQuery(query.to_string()));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_filter_apps(move |idx| {
            let filter = match idx {
                1 => AppFilterType::System,
                2 => AppFilterType::ThirdParty,
                _ => AppFilterType::All,
            };
            let _ = tx.send(AppIntent::LoadPackages(filter));
        });
    }

    {
        let tx = tx.clone();
        let ui_handle = app_weak.clone();
        app.global::<App>().on_refresh_apps(move || {
            if let Some(ui) = ui_handle.upgrade() {
                let filter = match ui.global::<App>().get_app_filter_tab() {
                    1 => AppFilterType::System,
                    2 => AppFilterType::ThirdParty,
                    _ => AppFilterType::All,
                };
                let _ = tx.send(AppIntent::LoadPackages(filter));
            }
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_launch_app(move |pkg| {
            let _ = tx.send(AppIntent::LaunchApp(pkg.to_string()));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_stop_app(move |pkg| {
            let _ = tx.send(AppIntent::StopApp(pkg.to_string()));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_clear_app(move |pkg| {
            let _ = tx.send(AppIntent::ClearAppData(pkg.to_string()));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_uninstall_app(move |pkg| {
            let _ = tx.send(AppIntent::UninstallApp(pkg.to_string()));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_extract_apk(move |remote_path, pkg| {
            let _ = tx.send(AppIntent::ExtractApk {
                remote_apk_path: remote_path.to_string(),
                package_name: pkg.to_string(),
            });
        });
    }

    // Logcat 日志交互
    {
        let tx = tx.clone();
        app.global::<App>().on_set_log_recording(move |enabled| {
            let _ = tx.send(AppIntent::SetLogRecording(enabled));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>()
            .on_open_log_recording_directory(move || {
                let _ = tx.send(AppIntent::OpenLogRecordingDirectory);
            });
    }

    {
        let tx = tx.clone();
        let ui_handle = app_weak.clone();
        app.global::<App>().on_toggle_logcat(move || {
            if let Some(ui) = ui_handle.upgrade() {
                if ui.global::<App>().get_is_logcat_active() {
                    let _ = tx.send(AppIntent::StopLogcatStream);
                } else {
                    let _ = tx.send(AppIntent::StartLogcatStream);
                }
            }
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_clear_logcat_view(move || {
            let _ = tx.send(AppIntent::ClearLogs);
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_filter_log_level(move |idx| {
            let level = match idx {
                1 => Some(LogLevel::Verbose),
                2 => Some(LogLevel::Debug),
                3 => Some(LogLevel::Info),
                4 => Some(LogLevel::Warn),
                5 => Some(LogLevel::Error),
                _ => None,
            };
            let _ = tx.send(AppIntent::SetLogLevelFilter(level));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_search_logs(move |query| {
            let _ = tx.send(AppIntent::SetLogSearchQuery(query.to_string()));
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_add_log_filter(move |field, value| {
            let _ = tx.send(AppIntent::AddLogFilter {
                field: field.to_string(),
                value: value.to_string(),
            });
        });
    }

    // 拖拽 APK 释放安装交互
    {
        app.global::<App>().on_check_has_apk(|data| {
            if let Ok(paths) = data.file_paths() {
                for path in paths {
                    if let Some(ext) = path.extension() {
                        if ext.to_string_lossy().eq_ignore_ascii_case("apk") {
                            return true;
                        }
                    }
                }
            }
            false
        });
    }

    {
        let tx = tx.clone();
        app.global::<App>().on_handle_apk_drop(move |data| {
            if let Ok(paths) = data.file_paths() {
                for path in paths {
                    if let Some(ext) = path.extension() {
                        if ext.to_string_lossy().eq_ignore_ascii_case("apk") {
                            let _ = tx.send(AppIntent::InstallApk(path.to_path_buf()));
                        }
                    }
                }
            }
        });
    }

    // 无线调试
    {
        let tx = tx.clone();
        let ui_handle = app_weak.clone();
        app.global::<App>().on_wireless_connect(move |address| {
            let address = address.to_string();
            let history = remember_wireless_address(&address);
            if let Some(ui) = ui_handle.upgrade() {
                ui.global::<App>()
                    .set_wireless_history(ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                        history,
                    ))));
            }
            let _ = tx.send(AppIntent::WirelessConnect(address));
        });
    }
    {
        let tx = tx.clone();
        app.global::<App>().on_wireless_disconnect(move |address| {
            let _ = tx.send(AppIntent::WirelessDisconnect(address.to_string()));
        });
    }
    {
        let tx = tx.clone();
        let ui_handle = app_weak.clone();
        app.global::<App>().on_enable_wireless_mode(move || {
            // 立即用概览页已知的 Wi-Fi IP 预填地址框，切换成功后即可直接连接
            if let Some(ui) = ui_handle.upgrade() {
                if let Some(ip) = parse_wifi_ip(ui.global::<App>().get_device_wifi_ip().as_str()) {
                    ui.global::<App>()
                        .set_wireless_address(format!("{ip}:5555").into());
                }
            }
            let _ = tx.send(AppIntent::EnableWirelessMode);
        });
    }

    // 快捷工具与导出
    {
        let tx = tx.clone();
        app.global::<App>().on_capture_screenshot(move || {
            let _ = tx.send(AppIntent::CaptureScreenshot);
        });
    }
    {
        let tx = tx.clone();
        app.global::<App>()
            .on_capture_screen_recording(move |seconds| {
                let _ = tx.send(AppIntent::CaptureScreenRecording {
                    seconds: seconds as u32,
                });
            });
    }
    {
        let tx = tx.clone();
        app.global::<App>().on_capture_bugreport(move || {
            let _ = tx.send(AppIntent::CaptureBugreport);
        });
    }
    {
        let tx = tx.clone();
        app.global::<App>().on_export_performance_csv(move || {
            let _ = tx.send(AppIntent::ExportPerformanceCsv);
        });
    }
    {
        let tx = tx.clone();
        app.global::<App>().on_export_filtered_logs(move || {
            let _ = tx.send(AppIntent::ExportFilteredLogs);
        });
    }

    // 文件管理器右键操作
    {
        let tx = tx.clone();
        app.global::<App>().on_confirm_delete_file(move |path| {
            let _ = tx.send(AppIntent::DeleteRemoteFile(path.to_string()));
        });
    }
    {
        let tx = tx.clone();
        app.global::<App>()
            .on_rename_file(move |old_path, new_name| {
                let _ = tx.send(AppIntent::RenameRemoteFile {
                    old_path: old_path.to_string(),
                    new_name: new_name.to_string(),
                });
            });
    }
    {
        let tx = tx.clone();
        app.global::<App>().on_create_directory(move |name| {
            let _ = tx.send(AppIntent::CreateRemoteDirectory(name.to_string()));
        });
    }
    {
        let tx = tx.clone();
        app.global::<App>().on_cancel_transfer(move || {
            let _ = tx.send(AppIntent::CancelTransfer);
        });
    }

    // 点击参数卡片复制到系统剪贴板
    {
        let tx = tx.clone();
        app.global::<App>().on_copy_clipboard(move |text| {
            let val = text.to_string();
            if !val.is_empty() && val != "未知" {
                match arboard::Clipboard::new() {
                    Ok(mut cb) => {
                        if let Err(e) = cb.set_text(&val) {
                            tracing::warn!("写入系统剪贴板失败: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("初始化剪贴板实例失败: {}", e);
                    }
                }
                let _ = tx.send(AppIntent::SetStatusMessage {
                    message: format!("已复制到剪贴板: {}", val),
                    is_busy: false,
                });
            }
        });
    }
}

/// scrcpy 本体路径变更时立即持久化（手输与文件夹选择共用）
fn persist_scrcpy_path(app: &AppWindow) {
    let handle = app.as_weak();
    app.global::<App>().on_scrcpy_path_changed(move || {
        let Some(ui) = handle.upgrade() else { return };
        let mut config = crate::app_config::load();
        let path = ui.global::<App>().get_scrcpy_path().to_string();
        config.scrcpy_path = (!path.is_empty()).then_some(path);
        crate::app_config::save(&config);
    });
}

/// 解析概览页展示的 Wi-Fi IP 文本，占位文本（如 "-"、"无法获取"）返回 None
fn parse_wifi_ip(text: &str) -> Option<std::net::Ipv4Addr> {
    text.trim().parse::<std::net::Ipv4Addr>().ok()
}

/// 无线地址连接后写入本地历史（去重、最新在前、上限 8 条）
fn remember_wireless_address(address: &str) -> Vec<SharedString> {
    let address = address.trim();
    if address.is_empty() {
        return crate::app_config::load()
            .wireless_history
            .into_iter()
            .map(Into::into)
            .collect();
    }
    let mut config = crate::app_config::load();
    config.wireless_history.retain(|item| item != address);
    config.wireless_history.insert(0, address.to_string());
    config.wireless_history.truncate(8);
    crate::app_config::save(&config);
    config
        .wireless_history
        .into_iter()
        .map(Into::into)
        .collect()
}
