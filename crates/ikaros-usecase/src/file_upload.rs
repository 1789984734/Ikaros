use super::FileUseCases;
use ikaros_domain::{DomainError, TransferProgress};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::{self, UnboundedSender};

#[cfg(test)]
#[path = "file_upload_tests.rs"]
mod tests;

struct UploadFile {
    local_path: PathBuf,
    remote_path: String,
    size: u64,
}

#[derive(Default)]
struct UploadPlan {
    directories: Vec<String>,
    files: Vec<UploadFile>,
    total_bytes: u64,
}

impl UploadPlan {
    async fn collect(local_paths: &[PathBuf], remote_directory: &str) -> Result<Self, DomainError> {
        let mut pending = local_paths
            .iter()
            .rev()
            .map(|path| Ok((path.clone(), remote_child(remote_directory, path)?)))
            .collect::<Result<Vec<_>, DomainError>>()?;
        let mut plan = Self::default();
        while let Some((local_path, remote_path)) = pending.pop() {
            let metadata = tokio::fs::symlink_metadata(&local_path)
                .await
                .map_err(|error| local_error(&local_path, error))?;
            if metadata.is_dir() {
                plan.directories.push(remote_path.clone());
                let mut entries = tokio::fs::read_dir(&local_path)
                    .await
                    .map_err(|error| local_error(&local_path, error))?;
                while let Some(entry) = entries
                    .next_entry()
                    .await
                    .map_err(|error| local_error(&local_path, error))?
                {
                    let path = entry.path();
                    pending.push((path.clone(), remote_child(&remote_path, &path)?));
                }
            } else if metadata.is_file() {
                plan.total_bytes = plan.total_bytes.saturating_add(metadata.len());
                plan.files.push(UploadFile {
                    local_path,
                    remote_path,
                    size: metadata.len(),
                });
            } else {
                return Err(DomainError::FileSyncFailed(format!(
                    "不支持上传符号链接或特殊文件: {}",
                    local_path.display()
                )));
            }
        }
        Ok(plan)
    }
}

fn remote_child(directory: &str, path: &Path) -> Result<String, DomainError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            DomainError::FileSyncFailed(format!("无法获取有效的文件名: {}", path.display()))
        })?;
    Ok(format!("{}/{name}", directory.trim_end_matches('/')))
}

fn local_error(path: &Path, error: std::io::Error) -> DomainError {
    DomainError::FileSyncFailed(format!("读取本地路径 {} 失败: {error}", path.display()))
}

impl FileUseCases {
    /// 顺序上传选中的文件和目录，保留顶层目录、子目录及空目录。
    pub async fn push_paths(
        &self,
        serial: &str,
        local_paths: &[PathBuf],
        remote_directory: &str,
        progress_tx: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        let plan = UploadPlan::collect(local_paths, remote_directory).await?;
        for directory in &plan.directories {
            self.sync_repo.make_directory(serial, directory).await?;
        }
        let mut progress = TransferProgress {
            filename: remote_directory.into(),
            transferred_bytes: 0,
            total_bytes: plan.total_bytes,
            speed_bytes_per_sec: 0,
            is_completed: false,
        };
        for (index, file) in plan.files.iter().enumerate() {
            progress.filename =
                format!("({}/{}) {}", index + 1, plan.files.len(), file.remote_path);
            if let Some(tx) = &progress_tx {
                let _ = tx.send(progress.clone());
            }
            self.push_planned_file(serial, file, &mut progress, progress_tx.as_ref())
                .await?;
            progress.transferred_bytes = progress.transferred_bytes.saturating_add(file.size);
        }
        progress.is_completed = true;
        if let Some(tx) = progress_tx {
            let _ = tx.send(progress);
        }
        Ok(())
    }

    async fn push_planned_file(
        &self,
        serial: &str,
        file: &UploadFile,
        batch: &mut TransferProgress,
        progress_tx: Option<&UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let transfer = self.sync_repo.push_file(
            serial,
            &file.local_path,
            &file.remote_path,
            progress_tx.map(|_| tx),
        );
        tokio::pin!(transfer);
        loop {
            tokio::select! {
                biased;
                Some(progress) = rx.recv() => {
                    batch.speed_bytes_per_sec = progress.speed_bytes_per_sec;
                    if let Some(tx) = progress_tx {
                        let _ = tx.send(TransferProgress {
                            transferred_bytes: batch.transferred_bytes
                                .saturating_add(progress.transferred_bytes.min(file.size)),
                            ..batch.clone()
                        });
                    }
                }
                result = &mut transfer => {
                    return result.map_err(|error| DomainError::FileSyncFailed(format!(
                        "上传 {} 失败: {error}", file.remote_path
                    )));
                }
            }
        }
    }
}
