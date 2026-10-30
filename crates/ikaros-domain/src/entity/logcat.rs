use serde::{Deserialize, Serialize};

/// Android 日志级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LogLevel {
    Verbose = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Fatal = 5,
}

impl LogLevel {
    pub fn from_char(c: char) -> Self {
        match c {
            'V' | 'v' => Self::Verbose,
            'D' | 'd' => Self::Debug,
            'I' | 'i' => Self::Info,
            'W' | 'w' => Self::Warn,
            'E' | 'e' => Self::Error,
            'F' | 'f' => Self::Fatal,
            _ => Self::Debug,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verbose => "V",
            Self::Debug => "D",
            Self::Info => "I",
            Self::Warn => "W",
            Self::Error => "E",
            Self::Fatal => "F",
        }
    }
}

/// 单条结构化 Logcat 日志记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// 时间戳 (如 "10-27 19:20:30.123")
    pub timestamp: String,
    /// 进程 PID
    pub pid: u32,
    /// 线程 TID
    pub tid: u32,
    /// 日志来源的应用包名；无法可靠确定时为空。
    #[serde(default)]
    pub package_name: Option<String>,
    /// 日志来源进程，用于识别没有应用包名的系统服务。
    #[serde(default)]
    pub process_name: Option<String>,
    /// 日志级别
    pub level: LogLevel,
    /// 日志标签 (Tag)
    pub tag: String,
    /// 消息内容
    pub message: String,
}
