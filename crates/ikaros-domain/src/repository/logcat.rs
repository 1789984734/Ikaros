use crate::entity::LogEntry;
use crate::error::DomainError;
use async_trait::async_trait;
use tokio::sync::{mpsc::Sender, watch::Receiver};

/// Logcat 日志仓储契约
#[async_trait]
pub trait LogcatRepository: Send + Sync {
    /// 持续拉取设备上的实时 Logcat 字节流并转化为结构化日志行
    async fn start_logcat_stream(
        &self,
        serial: &str,
        tx: Sender<LogEntry>,
        stop_rx: Receiver<bool>,
    ) -> Result<(), DomainError>;

    /// 清空设备当前的 logcat 环形缓冲区 (logcat -c)
    async fn clear_logcat(&self, serial: &str) -> Result<(), DomainError>;
}
