/// Raw counters from a single read-only device sample.
#[derive(Debug, Clone, Default)]
pub struct PerformanceReading {
    pub cpu: Option<CpuCounters>,
    pub memory_total_kb: Option<u64>,
    pub memory_available_kb: Option<u64>,
    pub battery_percent: Option<f64>,
    pub battery_temperature_c: Option<f64>,
    pub process_ids: Vec<u32>,
    pub process_list_available: bool,
    pub processes: Vec<ProcessCounters>,
    pub app_pss_kb: Option<u64>,
    pub frames: Vec<FrameCounters>,
    /// 除 lo 外所有网卡的累计接收字节数
    pub network_rx_bytes: Option<u64>,
    /// 除 lo 外所有网卡的累计发送字节数
    pub network_tx_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CpuCounters {
    pub total: u64,
    pub idle: u64,
}

#[derive(Debug, Clone)]
pub struct ProcessCounters {
    pub pid: u32,
    pub started: u64,
    pub ticks: u64,
}

#[derive(Debug, Clone)]
pub struct FrameCounters {
    pub pid: u32,
    pub since: u64,
    pub rendered: u64,
    pub janky: u64,
}

/// Percentages use the whole device's CPU capacity as 100%.
#[derive(Debug, Clone, Default)]
pub struct PerformanceSample {
    pub elapsed_seconds: f64,
    pub cpu_percent: Option<f64>,
    pub memory_total_kb: Option<u64>,
    pub memory_used_kb: Option<u64>,
    pub battery_percent: Option<f64>,
    pub battery_temperature_c: Option<f64>,
    pub process_ids: Vec<u32>,
    pub process_list_available: bool,
    pub app_cpu_percent: Option<f64>,
    pub app_pss_kb: Option<u64>,
    pub app_ui_fps: Option<f64>,
    pub app_jank_percent: Option<f64>,
    pub network_rx_rate_bps: Option<f64>,
    pub network_tx_rate_bps: Option<f64>,
}
