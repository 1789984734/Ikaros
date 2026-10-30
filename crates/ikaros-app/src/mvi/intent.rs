use ikaros_domain::{AppFilterType, AppInfo, Device, DeviceDetail, LogEntry, LogLevel, RemoteFile, TransferProgress};
use crate::scrcpy::ScrcpyLaunchOptions;

/// MVI 中的 Intent（意图 / 行为）
#[derive(Debug, Clone)]
pub enum AppIntent {
    TogglePerformance,
    SelectPerformancePackage(String),
    PerformancePackagesLoaded { serial: String, result: Result<Vec<String>, String> },
    PerformanceSampled { generation: u64, sample: ikaros_domain::PerformanceSample },
    PerformanceFailed { generation: u64, error: String },
    /// 刷新在线设备列表
    RefreshDevices,
    /// 收到设备列表变化（如热插拔或初次拉取成功）
    DevicesUpdated(Vec<Device>),
    /// 用户在界面上选中了某台设备
    SelectDevice(usize),
    /// 加载指定设备的详情信息
    LoadDeviceDetail(String),
    /// 设备详情加载完成
    DeviceDetailLoaded(DeviceDetail),
    /// 切换标签面板
    SwitchTab(i32),
    /// 执行 Shell 命令行
    ExecuteShell(String),
    /// 中断当前正在流式输出的终端命令
    InterruptShell,
    /// Shell 输出更新
    ShellOutputReceived(String),
    /// 流式 Shell 命令执行结束（断开或 EOF），generation 用于作废过期结果
    ShellCommandFinished { generation: u64, error: Option<String> },
    /// 状态提示消息变更
    SetStatusMessage { message: String, is_busy: bool },

    // ========== 投屏交互意图 ==========
    /// 使用 UI 收集的参数启动外部 scrcpy 客户端
    StartMirroring(ScrcpyLaunchOptions),
    /// 停止当前选中设备的投屏
    StopMirroring,
    /// 指定设备的 scrcpy 进程退出，忽略已被替换会话的通知。
    ScrcpyExited { serial: String, pid: u32 },

    // ========== 文件管理交互意图 ==========
    /// 浏览指定远程目录
    BrowseDirectory(String),
    /// 目录数据加载完成
    DirectoryLoaded {
        current_path: String,
        files: Vec<RemoteFile>,
    },
    /// 返回上一级目录
    NavigateUpDirectory,
    /// 进入某个子目录
    EnterDirectory(String),
    /// 拉取远程文件到本地
    PullRemoteFile {
        remote_path: String,
        local_target: String,
    },
    /// 推送本地文件到远程
    PushLocalFile {
        local_path: String,
        remote_target: String,
    },
    /// 删除远程文件或目录
    DeleteRemoteFile(String),
    /// 重命名远程文件或目录（同目录内改名）
    RenameRemoteFile { old_path: String, new_name: String },
    /// 在当前目录创建新文件夹
    CreateRemoteDirectory(String),
    /// 远程文件操作（删除/重命名/建目录）完成
    FileOpCompleted {
        action: String,
        result: Result<(), String>,
        refresh_dir: Option<String>,
    },
    /// 传输进度更新
    TransferProgressUpdated(TransferProgress),
    /// 设置文件列表排序键（点击同一列时切换升降序）
    SetFileSort(i32),
    /// 请求取消当前进行中的文件传输
    CancelTransfer,
    /// 传输任务已结束（成功/失败/取消），store 释放取消句柄
    TransferSettled,

    // ========== 应用包管理交互意图 ==========
    /// 加载应用列表
    LoadPackages(AppFilterType),
    /// 应用列表加载完成
    PackagesLoaded {
        serial: String,
        filter: AppFilterType,
        result: Result<Vec<AppInfo>, String>,
    },
    /// 设置应用名或包名搜索词
    SetAppSearchQuery(String),
    /// 启动应用
    LaunchApp(String),
    /// 强制停止应用
    StopApp(String),
    /// 清除应用数据
    ClearAppData(String),
    /// 卸载应用
    UninstallApp(String),
    /// 指定设备的卸载操作完成。
    AppUninstalled {
        serial: String,
        package_name: String,
        result: Result<(), String>,
    },
    /// 提取/备份 APK 到本地
    ExtractApk {
        remote_apk_path: String,
        package_name: String,
    },
    /// 安装本地 APK 文件（例如拖拽 APK 释放安装）
    InstallApk(std::path::PathBuf),

    // ========== 无线调试交互意图 ==========
    /// 连接一台已开启无线调试的远程设备（ip:port）
    WirelessConnect(String),
    /// 断开一台远程无线设备
    WirelessDisconnect(String),
    /// 将当前 USB 设备切换到 TCP 无线模式并自动回连
    EnableWirelessMode,

    // ========== 设备快捷工具意图 ==========
    /// 截取当前设备屏幕并保存到本地
    CaptureScreenshot,
    /// 录制指定秒数的设备屏幕
    CaptureScreenRecording { seconds: u32 },
    /// 生成 zip 格式 Bug 报告并保存到本地
    CaptureBugreport,

    // ========== 导出意图 ==========
    /// 导出性能监控历史为 CSV
    ExportPerformanceCsv,
    /// 导出当前筛选后的日志为 .log 文件
    ExportFilteredLogs,

    // ========== Logcat 日志交互意图 ==========
    /// 启动 Logcat 日志流
    StartLogcatStream,
    /// 停止 Logcat 日志流
    StopLogcatStream,
    /// 开启本地记录，或关闭并清理本功能生成的日志文件。
    SetLogRecording(bool),
    /// 后台记录任务返回实际状态。
    LogRecordingUpdated { enabled: bool, error: Option<String> },
    OpenLogRecordingDirectory,
    /// 接收到同一会话的一批日志
    LogEntriesReceived { generation: u64, entries: Vec<LogEntry> },
    /// 日志流结束或连接失败
    LogcatStreamEnded { generation: u64, error: Option<String> },
    /// 设备日志清理完成后，按当前会话决定是否恢复抓取
    LogcatCleared { generation: u64, resume: bool, error: Option<String> },
    /// 设置日志过滤级别
    SetLogLevelFilter(Option<LogLevel>),
    /// 设置关键字搜索
    SetLogSearchQuery(String),
    /// Append a structured filter from a log cell's context menu.
    AddLogFilter { field: String, value: String },
    /// 清空日志
    ClearLogs,
}
