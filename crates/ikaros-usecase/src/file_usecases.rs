use ikaros_domain::{DomainError, RemoteFile, SyncRepository, TransferProgress};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// 文件浏览与传输用例
pub struct FileUseCases {
    sync_repo: Arc<dyn SyncRepository>,
}

impl FileUseCases {
    pub fn new(sync_repo: Arc<dyn SyncRepository>) -> Self {
        Self { sync_repo }
    }

    /// 用例：浏览指定目录，排序（目录排在最前，按名称升序）
    pub async fn browse_directory(
        &self,
        serial: &str,
        path: &str,
    ) -> Result<Vec<RemoteFile>, DomainError> {
        let mut files = self.sync_repo.list_directory(serial, path).await?;
        // 排序规则：目录优先排在前面，同类型按字母排序
        files.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(files)
    }

    /// 用例：从手机拉取文件到本地
    pub async fn pull_file(
        &self,
        serial: &str,
        remote_path: &str,
        local_path: &Path,
        progress_tx: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        self.sync_repo
            .pull_file(serial, remote_path, local_path, progress_tx)
            .await
    }

    /// 用例：推送本地文件到手机
    pub async fn push_file(
        &self,
        serial: &str,
        local_path: &Path,
        remote_path: &str,
        progress_tx: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        self.sync_repo
            .push_file(serial, local_path, remote_path, progress_tx)
            .await
    }

    /// 用例：删除远程文件或目录
    pub async fn delete_path(&self, serial: &str, remote_path: &str) -> Result<(), DomainError> {
        self.sync_repo.delete_path(serial, remote_path).await
    }

    /// 用例：重命名远程文件或目录
    pub async fn rename_path(
        &self,
        serial: &str,
        remote_path: &str,
        new_name: &str,
    ) -> Result<(), DomainError> {
        self.sync_repo
            .rename_path(serial, remote_path, new_name)
            .await
    }

    /// 用例：创建远程目录（含父目录）
    pub async fn make_directory(&self, serial: &str, remote_path: &str) -> Result<(), DomainError> {
        self.sync_repo.make_directory(serial, remote_path).await
    }
}
