use crate::{mvi::AppIntent, scrcpy_ui, App, AppWindow};
use ikaros_domain::{AppFilterType, LogLevel};
use slint::{ComponentHandle, ModelRc, SharedString};
use std::path::{Path, PathBuf};
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
                let app = ui.global::<App>();
                let serial = app.get_current_serial().to_string();
                let options = scrcpy_ui::current_preferences(&ui)
                    .launch_options(app.get_scrcpy_path().to_string());
                let _ = tx.send(AppIntent::StartMirroring { serial, options });
            }
        });
    }

    {
        let tx = tx.clone();
        let ui_handle = app_weak.clone();
        app.global::<App>().on_stop_mirroring(move || {
            if let Some(ui) = ui_handle.upgrade() {
                let serial = ui.global::<App>().get_current_serial().to_string();
                let _ = tx.send(AppIntent::StopMirroring { serial });
            }
        });
    }

    // 投屏本体路径: 打开系统原生文件夹选择器 (Finder) 回填路径框并触发持久化
    bind_scrcpy_path_browse(app, pick_scrcpy_folder);

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

    // 拖拽 APK 释放安装交互由 apk_drag 模块直接接入原生窗口事件, 不经过 Slint 回调

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

fn pick_scrcpy_folder(current: &Path) -> Option<PathBuf> {
    // 从当前已填路径出发定位起始目录, 减少重复导航
    let mut dialog = rfd::FileDialog::new().set_title("选择 scrcpy 所在文件夹");
    let start_dir = if current.is_dir() {
        Some(current.to_path_buf())
    } else {
        current
            .parent()
            .map(Path::to_path_buf)
            .filter(|p| p.is_dir())
    };
    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_folder()
}

/// 绑定「浏览…」按钮回调：回填所选目录后复用变更回调持久化
fn bind_scrcpy_path_browse(
    app: &AppWindow,
    pick_folder: impl Fn(&Path) -> Option<PathBuf> + 'static,
) {
    let ui_handle = app.as_weak();
    app.global::<App>().on_browse_scrcpy_path(move || {
        let Some(ui) = ui_handle.upgrade() else {
            return;
        };
        let current = PathBuf::from(ui.global::<App>().get_scrcpy_path().to_string());
        if let Some(folder) = pick_folder(&current) {
            ui.global::<App>()
                .set_scrcpy_path(folder.to_string_lossy().to_string().into());
            ui.global::<App>().invoke_scrcpy_path_changed();
        }
    });
}

/// scrcpy 本体路径变更时立即持久化（手输与文件夹选择共用）
fn persist_scrcpy_path(app: &AppWindow) {
    bind_scrcpy_path_persist(app, save_scrcpy_path);
}

fn bind_scrcpy_path_persist(app: &AppWindow, save: impl Fn(&str) + 'static) {
    let handle = app.as_weak();
    app.global::<App>().on_scrcpy_path_changed(move || {
        let Some(ui) = handle.upgrade() else { return };
        let path = ui.global::<App>().get_scrcpy_path().to_string();
        save(&path);
    });
}

fn save_scrcpy_path(path: &str) {
    crate::app_config::update(|config| {
        apply_scrcpy_path(config, path);
    });
}

fn apply_scrcpy_path(config: &mut crate::app_config::AppConfig, path: &str) -> bool {
    let next = (!path.trim().is_empty()).then(|| path.to_string());
    if config.scrcpy_path == next {
        return false;
    }
    config.scrcpy_path = next;
    true
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
    let config = crate::app_config::update(|config| {
        config.wireless_history.retain(|item| item != address);
        config.wireless_history.insert(0, address.to_string());
        config.wireless_history.truncate(8);
    });
    config
        .wireless_history
        .into_iter()
        .map(Into::into)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfig;
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    use slint::platform::{Platform, PlatformError, WindowAdapter};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct TestPlatform;

    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    #[test]
    fn browsing_and_editing_scrcpy_path_both_persist_config() {
        let _ = slint::platform::set_platform(Box::new(TestPlatform));
        let ui = AppWindow::new().unwrap();
        let persisted = Rc::new(RefCell::new(AppConfig::default()));
        let save_count = Rc::new(RefCell::new(0usize));

        let persisted_clone = persisted.clone();
        let count_clone = save_count.clone();
        bind_scrcpy_path_persist(&ui, move |path| {
            let mut config = persisted_clone.borrow_mut();
            if apply_scrcpy_path(&mut config, path) {
                *count_clone.borrow_mut() += 1;
            }
        });

        let picked_folder = Rc::new(RefCell::new(Some(PathBuf::from("/opt/homebrew/bin"))));
        let picker_result = picked_folder.clone();
        bind_scrcpy_path_browse(&ui, move |_| picker_result.borrow().clone());

        // 通过「浏览…」选中目录后，应更新 UI 并自动触发持久化
        ui.global::<App>().invoke_browse_scrcpy_path();
        assert_eq!(
            ui.global::<App>().get_scrcpy_path().as_str(),
            "/opt/homebrew/bin"
        );
        assert_eq!(
            persisted.borrow().scrcpy_path.as_deref(),
            Some("/opt/homebrew/bin")
        );
        assert_eq!(*save_count.borrow(), 1);

        // 取消文件选择时，应保留原路径且不重复写入
        *picked_folder.borrow_mut() = None;
        ui.global::<App>().invoke_browse_scrcpy_path();
        assert_eq!(
            persisted.borrow().scrcpy_path.as_deref(),
            Some("/opt/homebrew/bin")
        );
        assert_eq!(*save_count.borrow(), 1);

        // 手动修改路径并触发变更回调，也应同步更新或清空配置
        ui.global::<App>()
            .set_scrcpy_path("/Applications/scrcpy".into());
        ui.global::<App>().invoke_scrcpy_path_changed();
        assert_eq!(
            persisted.borrow().scrcpy_path.as_deref(),
            Some("/Applications/scrcpy")
        );
        assert_eq!(*save_count.borrow(), 2);

        // 相同路径再次触发不应重复保存
        ui.global::<App>().invoke_scrcpy_path_changed();
        assert_eq!(*save_count.borrow(), 2);

        // 清空或仅空白字符时，配置中的 scrcpy_path 应复位为 None
        ui.global::<App>().set_scrcpy_path("   ".into());
        ui.global::<App>().invoke_scrcpy_path_changed();
        assert_eq!(persisted.borrow().scrcpy_path, None);
        assert_eq!(*save_count.borrow(), 3);
    }
}
