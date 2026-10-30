use thiserror::Error;

/// 业务领域通用错误类型
#[derive(Error, Debug, Clone)]
pub enum DomainError {
    #[error("ADB 服务未连接或连接超时: {0}")]
    AdbConnectionFailed(String),

    #[error("未找到指定的设备: {0}")]
    DeviceNotFound(String),

    #[error("设备未授权，请在手机上点击'允许USB调试'")]
    DeviceUnauthorized,

    #[error("设备离线 (Offline)")]
    DeviceOffline,

    #[error("执行 Shell 命令失败: {0}")]
    ShellExecutionFailed(String),

    #[error("文件传输失败: {0}")]
    FileSyncFailed(String),

    #[error("Scrcpy 协议错误: {0}")]
    ScrcpyProtocolError(String),

    #[error("协议解析错误: {0}")]
    ParseError(String),

    #[error("未知错误: {0}")]
    Other(String),
}
