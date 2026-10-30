use ikaros_domain::{AppFilterType, AppInfo, Device, DeviceDetail, LogEntry, LogLevel, RemoteFile};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use ikaros_domain::PerformanceSample;

#[derive(Clone, Default)]
pub struct PerformanceState {
    pub is_running: bool,
    pub package: String,
    pub packages: Arc<Vec<String>>,
    pub history: Arc<VecDeque<PerformanceSample>>,
    pub message: String,
    pub has_error: bool,
}

/// MVI 中的 ViewState（不可变单一数据源状态）
#[derive(Clone)]
pub struct AppViewState {
    /// 当前在线的所有设备
    pub devices: Vec<Device>,
    /// 当前选中的设备索引
    pub selected_device_idx: usize,
    /// 当前选中设备的详细软硬件信息
    pub selected_device_detail: Option<DeviceDetail>,
    /// 当前激活的面板标签 (0: 概览, 1: 投屏, 2: 文件, 3: 应用, 4: 日志, 5: 终端, 6: 性能)
    pub active_tab: i32,
    /// 后台是否正在繁忙处理任务
    pub is_busy: bool,
    /// 状态栏提示信息
    pub status_message: String,
    /// 终端执行的 Shell 输出累积日志
    pub shell_output: String,
    /// 终端是否有流式命令正在执行（执行期间禁止并发提交）
    pub is_shell_busy: bool,

    /// 各设备正在运行的投屏会话，按设备序列号记录进程 PID。
    pub mirroring_pids: HashMap<String, u32>,

    /// 当前文件管理浏览路径 (默认 /sdcard)
    pub current_dir: String,
    /// 当前目录下的文件列表
    pub file_list: Vec<RemoteFile>,
    /// 文件传输进度或提示
    pub file_transfer_message: String,
    /// 文件传输进度 0.0~1.0；-1 表示不适用（如正在列目录）
    pub file_transfer_percent: f32,
    /// 文件列表排序键 (0: 名称 1: 大小 2: 修改时间)
    pub file_sort_key: i32,
    /// 文件列表排序方向（升序）
    pub file_sort_asc: bool,

    /// 已安装应用包列表
    pub app_list: Vec<AppInfo>,
    /// 当前应用过滤筛选模式
    pub app_filter: AppFilterType,
    /// 当前分类下的应用名或包名搜索词
    pub app_search_query: String,
    /// 应用操作提示信息
    pub app_status_message: String,

    /// 当前展示在界面上的匹配日志行（最多展示最新 300 条避免 UI 绘制过载）
    pub display_logs: Vec<LogEntry>,
    /// 日志级别筛选
    pub log_level_filter: Option<LogLevel>,
    /// 关键词搜索过滤
    pub log_search_query: String,
    /// 是否正在流式接收日志
    pub is_logcat_streaming: bool,
    /// 本地记录开关及后台操作状态，独立于界面的日志筛选。
    pub is_log_recording_enabled: bool,
    pub is_log_recording_busy: bool,
    pub log_recording_directory: String,
    pub log_recording_message: String,
    pub log_recording_has_error: bool,
    pub performance: PerformanceState,
}

impl Default for AppViewState {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            selected_device_idx: 0,
            selected_device_detail: None,
            active_tab: 0,
            is_busy: false,
            status_message: "就绪".to_string(),
            shell_output: String::new(),
            is_shell_busy: false,
            mirroring_pids: HashMap::new(),
            current_dir: "/sdcard".to_string(),
            file_list: Vec::new(),
            file_transfer_message: String::new(),
            file_transfer_percent: -1.0,
            file_sort_key: 0,
            file_sort_asc: true,
            app_list: Vec::new(),
            app_filter: AppFilterType::All,
            app_search_query: String::new(),
            app_status_message: "就绪".to_string(),
            display_logs: Vec::new(),
            log_level_filter: None,
            log_search_query: String::new(),
            is_logcat_streaming: false,
            is_log_recording_enabled: false,
            is_log_recording_busy: true,
            log_recording_directory: String::new(),
            log_recording_message: "正在检查本地日志...".to_string(),
            log_recording_has_error: false,
            performance: PerformanceState::default(),
        }
    }
}

impl AppViewState {
    /// 当前选中设备是否已有投屏会话。
    pub fn is_mirroring(&self) -> bool {
        self.current_device()
            .is_some_and(|device| self.mirroring_pids.contains_key(&device.serial))
    }

    /// 获取当前选中的设备
    pub fn current_device(&self) -> Option<&Device> {
        self.devices.get(self.selected_device_idx)
    }

    /// 获取当前选中设备的序列号
    pub fn current_serial(&self) -> String {
        self.current_device()
            .map(|d| d.serial.clone())
            .unwrap_or_else(|| "未连接".to_string())
    }
}
