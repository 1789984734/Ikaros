use std::sync::Arc;
use tokio::sync::{mpsc::Sender, watch::Receiver};
use ikaros_domain::{DomainError, LogEntry, LogcatRepository};

/// Logcat 日志处理用例
pub struct LogcatUseCases {
    repo: Arc<dyn LogcatRepository>,
}

impl LogcatUseCases {
    pub fn new(repo: Arc<dyn LogcatRepository>) -> Self {
        Self { repo }
    }

    /// 用例：开启后台异步日志拉取管道
    pub async fn stream_logs(
        &self,
        serial: &str,
        tx: Sender<LogEntry>,
        stop_rx: Receiver<bool>,
    ) -> Result<(), DomainError> {
        self.repo.start_logcat_stream(serial, tx, stop_rx).await
    }

    /// 用例：清空设备历史日志
    pub async fn clear_logs(&self, serial: &str) -> Result<(), DomainError> {
        self.repo.clear_logcat(serial).await
    }
}
