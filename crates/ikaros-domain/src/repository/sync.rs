use crate::entity::{RemoteFile, TransferProgress};
use crate::error::DomainError;
use async_trait::async_trait;
use std::path::Path;
use tokio::sync::mpsc::UnboundedSender;

/// ADB Sync 文件传输与浏览接口契约
#[async_trait]
pub trait SyncRepository: Send + Sync {
    /// 列出远程指定目录下的所有文件和子目录
    async fn list_directory(
        &self,
        serial: &str,
        path: &str,
    ) -> Result<Vec<RemoteFile>, DomainError>;

    /// 从远程设备拉取文件到本地计算机 (Pull)
    async fn pull_file(
        &self,
        serial: &str,
        remote_path: &str,
        local_path: &Path,
        progress_tx: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError>;

    /// 从本地计算机推送文件到远程设备 (Push)
    async fn push_file(
        &self,
        serial: &str,
        local_path: &Path,
        remote_path: &str,
        progress_tx: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError>;

    /// 删除远程文件或目录（递归）
    async fn delete_path(&self, serial: &str, remote_path: &str) -> Result<(), DomainError>;

    /// 重命名或移动远程文件/目录
    async fn rename_path(
        &self,
        serial: &str,
        remote_path: &str,
        new_name: &str,
    ) -> Result<(), DomainError>;

    /// 在远程设备上创建目录（含父目录）
    async fn make_directory(&self, serial: &str, remote_path: &str) -> Result<(), DomainError>;
}
