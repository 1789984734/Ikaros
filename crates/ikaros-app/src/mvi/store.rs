use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;
use ikaros_domain::LogEntry;
use ikaros_usecase::{AppUseCases, DeviceUseCases, FileUseCases, LogcatUseCases};
use ikaros_usecase::PerformanceUseCases;
use crate::mvi::intent::AppIntent;
use crate::mvi::state::AppViewState;
use crate::mvi::log_recording::LogRecorder;

const LOG_HISTORY_LIMIT: usize = 50_000;
const DISPLAY_LOG_LIMIT: usize = 300;

#[path = "performance.rs"]
mod performance;
#[path = "mirroring.rs"]
mod mirroring;
#[path = "device_store.rs"]
mod device_store;
#[path = "file_store.rs"]
mod file_store;
#[path = "package_store.rs"]
mod package_store;
#[path = "terminal_store.rs"]
mod terminal_store;
#[path = "logcat_store.rs"]
mod logcat_store;
#[cfg(test)]
#[path = "app_uninstall_tests.rs"]
mod app_uninstall_tests;

/// MVI 的核心调度器与状态机
/// 领域处理逻辑拆分在 device/file/package/terminal/logcat/performance 模块中。
pub struct AppStore {
    pub state: AppViewState,
    device_usecases: Arc<DeviceUseCases>,
    file_usecases: Arc<FileUseCases>,
    app_usecases: Arc<AppUseCases>,
    logcat_usecases: Arc<LogcatUseCases>,
    intent_sender: UnboundedSender<AppIntent>,
    /// 投屏进程的停止信号；设备与 PID 的对应关系由 ViewState 统一维护。
    scrcpy_stop_txs: HashMap<u32, watch::Sender<bool>>,
    logcat_stop_tx: Option<watch::Sender<bool>>,
    /// Full history stays in the store and is never copied into UI snapshots.
    logs_ring_buffer: VecDeque<LogEntry>,
    logcat_generation: u64,
    log_recorder: LogRecorder,
    performance_usecases: Arc<PerformanceUseCases>,
    performance_task: Option<tokio::task::JoinHandle<()>>,
    performance_packages_task: Option<tokio::task::JoinHandle<()>>,
    performance_generation: u64,
    performance_started_at: Option<std::time::Instant>,
    /// 终端流式命令的停止信号与代次（切换命令时旧结果作废）。
    shell_stop_tx: Option<watch::Sender<bool>>,
    shell_generation: u64,
    /// 进行中文件传输的取消标志；结束后由 TransferSettled 清空，并用于阻止并发传输。
    file_transfer_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl AppStore {
    pub fn new(
        device_usecases: Arc<DeviceUseCases>,
        file_usecases: Arc<FileUseCases>,
        app_usecases: Arc<AppUseCases>,
        logcat_usecases: Arc<LogcatUseCases>,
        intent_sender: UnboundedSender<AppIntent>,
        log_directory: PathBuf,
        performance_usecases: Arc<PerformanceUseCases>,
    ) -> Self {
        let log_recorder = LogRecorder::new(log_directory.clone(), intent_sender.clone());
        Self {
            state: AppViewState {
                status_message: "正在初始化 ADB 连接...".to_string(),
                log_recording_directory: log_directory.to_string_lossy().into_owned(),
                ..Default::default()
            },
            device_usecases,
            file_usecases,
            app_usecases,
            logcat_usecases,
            intent_sender,
            scrcpy_stop_txs: HashMap::new(),
            logcat_stop_tx: None,
            logs_ring_buffer: VecDeque::with_capacity(LOG_HISTORY_LIMIT),
            logcat_generation: 0,
            log_recorder,
            performance_usecases,
            performance_task: None,
            performance_packages_task: None,
            performance_generation: 0,
            performance_started_at: None,
            shell_stop_tx: None,
            shell_generation: 0,
            file_transfer_cancel: None,
        }
    }

    /// 统一接收并分发意图 (Intent)，各领域处理逻辑见对应 *_store 模块
    pub async fn dispatch(&mut self, intent: AppIntent) {
        match intent {
            // 性能监控
            AppIntent::TogglePerformance => self.toggle_performance(),
            AppIntent::SelectPerformancePackage(package) => self.select_performance_package(package),
            AppIntent::PerformancePackagesLoaded { serial, result } => {
                self.receive_performance_packages(serial, result);
            }
            AppIntent::PerformanceSampled { generation, sample } => {
                self.receive_performance_sample(generation, sample);
            }
            AppIntent::PerformanceFailed { generation, error } => {
                self.handle_performance_failed(generation, error);
            }
            AppIntent::ExportPerformanceCsv => self.on_export_performance_csv(),

            // 设备与概览、无线、快捷工具
            AppIntent::RefreshDevices => self.on_refresh_devices(),
            AppIntent::DevicesUpdated(devices) => self.on_devices_updated(devices),
            AppIntent::SelectDevice(index) => self.on_select_device(index),
            AppIntent::LoadDeviceDetail(serial) => self.on_load_device_detail(serial),
            AppIntent::DeviceDetailLoaded(detail) => self.on_device_detail_loaded(detail),
            AppIntent::SwitchTab(tab) => self.on_switch_tab(tab),
            AppIntent::WirelessConnect(address) => self.on_wireless_connect(address),
            AppIntent::WirelessDisconnect(address) => self.on_wireless_disconnect(address),
            AppIntent::EnableWirelessMode => self.on_enable_wireless_mode(),
            AppIntent::CaptureScreenshot => self.on_capture_screenshot(),
            AppIntent::CaptureScreenRecording { seconds } => self.on_capture_screen_recording(seconds),
            AppIntent::CaptureBugreport => self.on_capture_bugreport(),

            // 终端
            AppIntent::ExecuteShell(cmd) => self.on_execute_shell(cmd),
            AppIntent::InterruptShell => self.on_interrupt_shell(),
            AppIntent::ShellOutputReceived(output) => self.on_shell_output_received(output),
            AppIntent::ShellCommandFinished { generation, error } => {
                self.on_shell_command_finished(generation, error);
            }

            AppIntent::SetStatusMessage { message, is_busy } => {
                self.state.status_message = message;
                self.state.is_busy = is_busy;
            }

            // 投屏
            AppIntent::StartMirroring(options) => {
                self.start_mirroring(options).await;
            }
            AppIntent::StopMirroring => {
                self.stop_mirroring();
            }
            AppIntent::ScrcpyExited { serial, pid } => {
                self.scrcpy_exited(serial, pid);
            }

            // 文件管理
            AppIntent::BrowseDirectory(path) => self.on_browse_directory(path),
            AppIntent::DirectoryLoaded { current_path, files } => {
                self.on_directory_loaded(current_path, files);
            }
            AppIntent::EnterDirectory(dir_name) => self.on_enter_directory(dir_name),
            AppIntent::NavigateUpDirectory => self.on_navigate_up_directory(),
            AppIntent::PullRemoteFile { remote_path, local_target } => {
                self.on_pull_remote_file(remote_path, local_target);
            }
            AppIntent::PushLocalFile { local_path, remote_target } => {
                self.on_push_local_file(local_path, remote_target);
            }
            AppIntent::TransferProgressUpdated(prog) => self.on_transfer_progress_updated(prog),
            AppIntent::SetFileSort(key) => self.on_set_file_sort(key),
            AppIntent::CancelTransfer => self.on_cancel_transfer(),
            AppIntent::TransferSettled => self.on_transfer_settled(),
            AppIntent::DeleteRemoteFile(path) => self.on_delete_remote_file(path),
            AppIntent::RenameRemoteFile { old_path, new_name } => {
                self.on_rename_remote_file(old_path, new_name);
            }
            AppIntent::CreateRemoteDirectory(name) => self.on_create_remote_directory(name),
            AppIntent::FileOpCompleted { action, result, refresh_dir } => {
                self.on_file_op_completed(action, result, refresh_dir);
            }

            // 应用包管理
            AppIntent::LoadPackages(filter) => self.on_load_packages(filter),
            AppIntent::PackagesLoaded { serial, filter, result } => {
                self.on_packages_loaded(serial, filter, result);
            }
            AppIntent::SetAppSearchQuery(query) => self.on_set_app_search_query(query),
            AppIntent::LaunchApp(pkg) => self.on_launch_app(pkg),
            AppIntent::StopApp(pkg) => self.on_stop_app(pkg),
            AppIntent::ClearAppData(pkg) => self.on_clear_app_data(pkg),
            AppIntent::UninstallApp(pkg) => self.on_uninstall_app(pkg),
            AppIntent::AppUninstalled { serial, package_name, result } => {
                self.on_app_uninstalled(serial, package_name, result);
            }
            AppIntent::ExtractApk { remote_apk_path, package_name } => {
                self.on_extract_apk(remote_apk_path, package_name);
            }
            AppIntent::InstallApk(apk_path) => self.on_install_apk(apk_path),

            // Logcat
            AppIntent::StartLogcatStream => self.on_start_logcat_stream(),
            AppIntent::StopLogcatStream => self.stop_logcat_stream(),
            AppIntent::SetLogRecording(enabled) => self.on_set_log_recording(enabled),
            AppIntent::LogRecordingUpdated { enabled, error } => {
                self.on_log_recording_updated(enabled, error);
            }
            AppIntent::OpenLogRecordingDirectory => self.on_open_log_recording_directory(),
            AppIntent::LogEntriesReceived { generation, entries } => {
                self.on_log_entries_received(generation, entries);
            }
            AppIntent::LogcatStreamEnded { generation, error } => {
                self.on_logcat_stream_ended(generation, error);
            }
            AppIntent::SetLogLevelFilter(filter) => self.on_set_log_level_filter(filter),
            AppIntent::SetLogSearchQuery(query) => self.on_set_log_search_query(query),
            AppIntent::AddLogFilter { field, value } => self.on_add_log_filter(field, value),
            AppIntent::ClearLogs => self.on_clear_logs(),
            AppIntent::LogcatCleared { generation, resume, error } => {
                self.on_logcat_cleared(generation, resume, error);
            }
            AppIntent::ExportFilteredLogs => self.on_export_filtered_logs(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_adb::AdbClient;

    fn log_store() -> AppStore {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let adb = Arc::new(AdbClient::default());
        AppStore::new(
            Arc::new(DeviceUseCases::new(adb.clone())),
            Arc::new(FileUseCases::new(adb.clone())),
            Arc::new(AppUseCases::new(adb.clone())),
            Arc::new(LogcatUseCases::new(adb.clone())),
            tx,
            tempfile::tempdir().unwrap().path().join("logcat"),
            Arc::new(PerformanceUseCases::new(adb.clone())),
        )
    }

    fn log_entry(number: usize) -> LogEntry {
        LogEntry {
            timestamp: String::new(), pid: 1, tid: 1,
            package_name: None, process_name: None,
            level: if number == 16_000 { ikaros_domain::LogLevel::Error } else { ikaros_domain::LogLevel::Info },
            tag: "Needle".into(), message: number.to_string(),
        }
    }

    #[tokio::test]
    async fn package_and_keyword_filters_preserve_the_unfiltered_history() {
        let mut store = log_store();
        store.state.is_logcat_streaming = true;
        let mut first = log_entry(1);
        first.package_name = Some("com.example.first".into());
        first.message = "network timeout".into();
        let mut second = first.clone();
        second.package_name = Some("com.example.second".into());
        let mut success = first.clone();
        success.message = "network success".into();
        store.dispatch(AppIntent::LogEntriesReceived {
            generation: 0, entries: vec![first, second, success],
        }).await;
        store.dispatch(AppIntent::SetLogSearchQuery("package:com.example.first timeout".into())).await;
        assert_eq!(store.state.display_logs.len(), 1);
        assert_eq!(store.state.display_logs[0].package_name.as_deref(), Some("com.example.first"));
        store.dispatch(AppIntent::SetLogSearchQuery("package:com.example.second message:timeout".into())).await;
        assert_eq!(store.state.display_logs.len(), 1);
        assert_eq!(store.state.display_logs[0].package_name.as_deref(), Some("com.example.second"));
        store.dispatch(AppIntent::SetLogSearchQuery(String::new())).await;
        assert_eq!(store.state.display_logs.len(), 3);
        assert_eq!(store.logs_ring_buffer.len(), 3);
    }

    #[tokio::test]
    async fn successive_context_filters_append_to_the_current_query() {
        let mut store = log_store();
        store.state.is_logcat_streaming = true;
        let mut first = log_entry(1);
        first.package_name = Some("com.example.first".into());
        first.tag = "Network Worker".into();
        first.message = "network timeout".into();
        let mut second = first.clone();
        second.tag = "Other".into();
        let mut third = first.clone();
        third.package_name = Some("com.example.third".into());
        store.dispatch(AppIntent::LogEntriesReceived {
            generation: 0, entries: vec![first, second, third],
        }).await;
        store.dispatch(AppIntent::SetLogSearchQuery("timeout".into())).await;
        for (field, value) in [("package", "com.example.first"), ("tag", "Network Worker")] {
            store.dispatch(AppIntent::AddLogFilter { field: field.into(), value: value.into() }).await;
        }
        assert!(store.state.log_search_query.starts_with("timeout "));
        assert_eq!(store.state.display_logs.len(), 1);
        assert_eq!(store.state.display_logs[0].tag, "Network Worker");
        let query = store.state.log_search_query.clone();
        store.dispatch(AppIntent::AddLogFilter { field: "tag".into(), value: "Network Worker".into() }).await;
        assert_eq!(store.state.log_search_query, query);
        assert_eq!(store.logs_ring_buffer.len(), 3);
    }

    #[tokio::test]
    async fn context_level_filters_use_the_toolbar_level_and_keep_text_filters() {
        let mut store = log_store();
        store.state.log_search_query = "package:com.example timeout".into();
        for (value, expected) in [
            ("V", ikaros_domain::LogLevel::Verbose),
            ("D", ikaros_domain::LogLevel::Debug),
            ("I", ikaros_domain::LogLevel::Info),
            ("W", ikaros_domain::LogLevel::Warn),
            ("E", ikaros_domain::LogLevel::Error),
            ("F", ikaros_domain::LogLevel::Error),
        ] {
            store.dispatch(AppIntent::AddLogFilter { field: "level".into(), value: value.into() }).await;
            assert_eq!(store.state.log_level_filter, Some(expected));
            assert_eq!(store.state.log_search_query, "package:com.example timeout");
        }
        store.dispatch(AppIntent::AddLogFilter { field: "timestamp".into(), value: "now".into() }).await;
        assert_eq!(store.state.log_search_query, "package:com.example timeout");
    }

    #[tokio::test]
    async fn log_bursts_keep_bounded_history_and_filter_beyond_the_visible_tail() {
        let mut store = log_store();
        store.state.is_logcat_streaming = true;
        for start in (0..60_000).step_by(4096) {
            let entries = (start..(start + 4096).min(60_000)).map(log_entry).collect();
            store.dispatch(AppIntent::LogEntriesReceived { generation: 0, entries }).await;
        }
        assert_eq!(store.logs_ring_buffer.len(), LOG_HISTORY_LIMIT);
        assert_eq!(store.logs_ring_buffer.front().unwrap().message, "10000");
        assert_eq!(store.state.display_logs.len(), DISPLAY_LOG_LIMIT);
        assert_eq!(store.state.display_logs.first().unwrap().message, "59700");
        assert_eq!(store.state.display_logs.last().unwrap().message, "59999");
        store.dispatch(AppIntent::SetLogLevelFilter(Some(ikaros_domain::LogLevel::Error))).await;
        store.dispatch(AppIntent::SetLogSearchQuery("nEeDlE".into())).await;
        assert_eq!(store.state.display_logs.len(), 1);
        assert_eq!(store.state.display_logs[0].message, "16000");
    }

    #[tokio::test]
    async fn pause_and_replacement_reject_queued_logs_and_stale_completion() {
        let mut store = log_store();
        store.state.is_logcat_streaming = true;
        store.dispatch(AppIntent::StopLogcatStream).await;
        store.dispatch(AppIntent::LogEntriesReceived { generation: 0, entries: vec![log_entry(0)] }).await;
        assert!(store.logs_ring_buffer.is_empty());

        store.state.is_logcat_streaming = true;
        let status = store.state.status_message.clone();
        store.dispatch(AppIntent::LogcatStreamEnded { generation: 0, error: Some("old device".into()) }).await;
        store.dispatch(AppIntent::LogcatCleared { generation: 0, resume: true, error: None }).await;
        assert!(store.state.is_logcat_streaming);
        assert_eq!(store.state.status_message, status);
        store.dispatch(AppIntent::LogEntriesReceived { generation: 1, entries: vec![log_entry(1)] }).await;
        assert_eq!(store.state.display_logs[0].message, "1");
        store.dispatch(AppIntent::LogcatStreamEnded { generation: 1, error: None }).await;
        assert!(!store.state.is_logcat_streaming);
    }

    #[tokio::test]
    async fn recording_failure_pauses_capture_and_keeps_cleanup_available() {
        let mut store = log_store();
        store.state.is_logcat_streaming = true;
        store.dispatch(AppIntent::LogRecordingUpdated {
            enabled: true, error: Some("disk full".into()),
        }).await;
        assert!(!store.state.is_logcat_streaming);
        assert!(store.state.is_log_recording_enabled);
        assert!(store.state.log_recording_has_error);
        assert!(!store.state.is_log_recording_busy);
        assert!(store.state.log_recording_message.contains("disk full"));
    }

    #[tokio::test]
    async fn test_store_install_apk_intent_unconnected() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let adb = Arc::new(AdbClient::default());
        let device_usecases = Arc::new(DeviceUseCases::new(adb.clone()));
        let file_usecases = Arc::new(FileUseCases::new(adb.clone()));
        let app_usecases = Arc::new(AppUseCases::new(adb.clone()));
        let logcat_usecases = Arc::new(LogcatUseCases::new(adb.clone()));

        let mut store = AppStore::new(
            device_usecases,
            file_usecases,
            app_usecases,
            logcat_usecases,
            tx,
            tempfile::tempdir().unwrap().path().join("logcat"),
            Arc::new(PerformanceUseCases::new(adb.clone())),
        );

        // Consume the independent recorder's initial state before checking the install result.
        assert!(matches!(rx.recv().await, Some(AppIntent::LogRecordingUpdated { .. })));
        // 设备未连接时发送 InstallApk
        store.dispatch(AppIntent::InstallApk(PathBuf::from("/tmp/test.apk"))).await;

        // 验证已接收到未连接的友好提示
        if let Some(AppIntent::SetStatusMessage { message, is_busy }) = rx.recv().await {
            assert!(message.contains("请先连接或选择一台设备"));
            assert!(!is_busy);
        } else {
            panic!("预期收到未连接设备提示");
        }
    }
}
