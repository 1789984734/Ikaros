use super::{AppIntent, AppStore};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::error;

impl AppStore {
    pub(super) fn on_browse_directory(&mut self, path: String) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            return;
        }
        let path = match ikaros_usecase::file_usecases::normalize_remote_path(
            &self.state.current_dir,
            &path,
        ) {
            Ok(path) => path,
            Err(error) => {
                self.state.status_message = error.to_string();
                return;
            }
        };
        self.clear_file_path_completions();
        if let Some(task) = self.file_browse_task.take() {
            task.abort();
        }
        self.file_browse_generation = self.file_browse_generation.wrapping_add(1);
        let generation = self.file_browse_generation;

        self.state.is_busy = true;
        if self.file_transfer_cancel.is_none() {
            self.state.file_transfer_message = format!("正在读取目录: {}...", path);
            self.state.file_transfer_percent = -1.0;
        }

        self.start_directory_load(serial, path, generation);
    }

    fn start_directory_load(&mut self, serial: String, dir_path: String, generation: u64) {
        let usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();

        self.file_browse_task = Some(tokio::spawn(async move {
            match usecases.browse_directory(&serial, &dir_path).await {
                Ok(files) => {
                    let _ = tx.send(AppIntent::DirectoryLoaded {
                        serial,
                        generation,
                        current_path: dir_path,
                        files,
                    });
                }
                Err(e) => {
                    error!("列出目录失败: {}", e);
                    let _ = tx.send(AppIntent::DirectoryLoadFailed {
                        serial,
                        generation,
                        message: format!("列出目录失败: {}", e),
                    });
                }
            }
        }));
    }

    pub(super) fn on_directory_loaded(
        &mut self,
        serial: String,
        generation: u64,
        current_path: String,
        files: Vec<ikaros_domain::RemoteFile>,
    ) {
        if serial != self.state.current_serial() || generation != self.file_browse_generation {
            return;
        }
        self.file_browse_task = None;
        self.clear_file_path_completions();
        self.state.is_busy = self.file_transfer_cancel.is_some();
        self.state.current_dir = current_path;
        if self.file_transfer_cancel.is_none() {
            self.state.file_transfer_message.clear();
            self.state.file_transfer_percent = -1.0;
        }
        self.state.file_list = files;
    }

    pub(super) fn on_directory_load_failed(
        &mut self,
        serial: String,
        generation: u64,
        message: String,
    ) {
        if serial != self.state.current_serial() || generation != self.file_browse_generation {
            return;
        }
        self.file_browse_task = None;
        self.state.is_busy = self.file_transfer_cancel.is_some();
        self.state.status_message = message.clone();
        if self.file_transfer_cancel.is_none() {
            self.state.file_transfer_message = message;
            self.state.file_transfer_percent = -1.0;
        }
    }

    pub(super) fn on_enter_directory(&mut self, dir_name: String) {
        let current = self.state.current_dir.trim_end_matches('/');
        let target = format!("{}/{}", current, dir_name);
        let _ = self.intent_sender.send(AppIntent::BrowseDirectory(target));
    }

    pub(super) fn on_navigate_up_directory(&mut self) {
        let current = self.state.current_dir.trim_end_matches('/');
        if current.is_empty() || current == "/sdcard" || current == "/" {
            return;
        }
        if let Some(pos) = current.rfind('/') {
            let parent = if pos == 0 { "/" } else { &current[..pos] };
            let _ = self
                .intent_sender
                .send(AppIntent::BrowseDirectory(parent.to_string()));
        }
    }

    pub(super) fn on_pull_remote_file(&mut self, remote_path: String, local_target: String) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            return;
        }
        if self.file_transfer_cancel.is_some() {
            self.reject_concurrent_transfer();
            return;
        }

        self.state.is_busy = true;
        self.state.file_transfer_message = format!("正在下载: {}...", remote_path);
        self.state.file_transfer_percent = 0.0;

        let usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        let local_path = PathBuf::from(local_target);
        let cancel = self.begin_transfer();

        tokio::spawn(async move {
            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
            let tx_prog = tx.clone();

            tokio::spawn(async move {
                while let Some(prog) = progress_rx.recv().await {
                    let _ = tx_prog.send(AppIntent::TransferProgressUpdated(prog));
                }
            });

            tokio::select! {
                result = usecases.pull_file(&serial, &remote_path, &local_path, Some(progress_tx)) => {
                    match result {
                        Ok(_) => {
                            let _ = tx.send(AppIntent::SetStatusMessage {
                                message: format!("下载成功保存至: {:?}", local_path),
                                is_busy: false,
                            });
                        }
                        Err(e) => {
                            error!("下载文件失败: {}", e);
                            let _ = tx.send(AppIntent::SetStatusMessage {
                                message: format!("下载失败: {}", e),
                                is_busy: false,
                            });
                        }
                    }
                }
                _ = wait_for_cancel(Arc::clone(&cancel)) => {
                    // 丢弃本地半截文件
                    let _ = tokio::fs::remove_file(&local_path).await;
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("已取消下载: {}", remote_path),
                        is_busy: false,
                    });
                }
            }
            let _ = tx.send(AppIntent::TransferSettled);
        });
    }

    pub(super) fn on_push_local_paths(
        &mut self,
        local_paths: Vec<PathBuf>,
        remote_directory: String,
    ) {
        let serial = self.state.current_serial();
        if serial == "未连接" || local_paths.is_empty() {
            return;
        }
        if self.file_transfer_cancel.is_some() {
            self.reject_concurrent_transfer();
            return;
        }

        self.state.is_busy = true;
        self.state.file_transfer_message = format!("正在准备上传至: {}...", remote_directory);
        self.state.file_transfer_percent = 0.0;

        let usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        let cancel = self.begin_transfer();
        tokio::spawn(async move {
            let message = upload_local_paths(
                &usecases,
                &serial,
                &local_paths,
                &remote_directory,
                &tx,
                cancel,
            )
            .await;
            let _ = tx.send(AppIntent::UploadCompleted { serial, message });
        });
    }

    pub(super) fn on_upload_completed(&mut self, serial: String, message: String) {
        self.on_transfer_settled();
        if self.state.current_serial() != serial {
            self.state.file_transfer_message.clear();
            self.state.file_transfer_percent = -1.0;
            return;
        }
        self.state.is_busy = false;
        self.state.file_transfer_percent = -1.0;
        self.state.file_transfer_message = message.clone();
        self.state.status_message = message;
        let _ = self
            .intent_sender
            .send(AppIntent::BrowseDirectory(self.state.current_dir.clone()));
    }

    pub(super) fn on_transfer_progress_updated(&mut self, prog: ikaros_domain::TransferProgress) {
        let speed_mb = prog.speed_bytes_per_sec as f64 / (1024.0 * 1024.0);
        let size_mb = prog.transferred_bytes as f64 / (1024.0 * 1024.0);
        self.state.file_transfer_message = format!(
            "{} (已传 {:.2} MB, 速度: {:.2} MB/s)",
            prog.filename, size_mb, speed_mb
        );
        self.state.file_transfer_percent = if prog.total_bytes > 0 {
            (prog.transferred_bytes as f32 / prog.total_bytes as f32).clamp(0.0, 1.0)
        } else if prog.is_completed {
            1.0
        } else {
            0.0
        };
    }

    /// 切换文件列表排序: 点击同一列切换方向, 切换列时重置为升序
    pub(super) fn on_set_file_sort(&mut self, key: i32) {
        if self.state.file_sort_key == key {
            self.state.file_sort_asc = !self.state.file_sort_asc;
        } else {
            self.state.file_sort_key = key;
            self.state.file_sort_asc = true;
        }
    }

    pub(super) fn on_delete_remote_file(&mut self, remote_path: String) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            return;
        }
        self.state.is_busy = true;
        self.state.file_transfer_message = format!("正在删除: {}...", remote_path);
        self.state.file_transfer_percent = -1.0;

        let usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        let current_dir = self.state.current_dir.clone();
        tokio::spawn(async move {
            let result = usecases.delete_path(&serial, &remote_path).await;
            let _ = tx.send(AppIntent::FileOpCompleted {
                action: "删除".into(),
                result: result.map_err(|e| e.to_string()),
                refresh_dir: Some(current_dir),
            });
        });
    }

    pub(super) fn on_rename_remote_file(&mut self, old_path: String, new_name: String) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            return;
        }
        self.state.is_busy = true;
        self.state.file_transfer_message = format!("正在重命名为: {}...", new_name);
        self.state.file_transfer_percent = -1.0;

        let usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        let current_dir = self.state.current_dir.clone();
        tokio::spawn(async move {
            let result = usecases.rename_path(&serial, &old_path, &new_name).await;
            let _ = tx.send(AppIntent::FileOpCompleted {
                action: "重命名".into(),
                result: result.map_err(|e| e.to_string()),
                refresh_dir: Some(current_dir),
            });
        });
    }

    pub(super) fn on_create_remote_directory(&mut self, dir_name: String) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            return;
        }
        let name = dir_name.trim().trim_matches('/').to_string();
        if name.is_empty() {
            return;
        }
        let target = format!("{}/{}", self.state.current_dir.trim_end_matches('/'), name);
        self.state.is_busy = true;
        self.state.file_transfer_message = format!("正在创建文件夹: {}...", name);
        self.state.file_transfer_percent = -1.0;

        let usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        let current_dir = self.state.current_dir.clone();
        tokio::spawn(async move {
            let result = usecases.make_directory(&serial, &target).await;
            let _ = tx.send(AppIntent::FileOpCompleted {
                action: "创建文件夹".into(),
                result: result.map_err(|e| e.to_string()),
                refresh_dir: Some(current_dir),
            });
        });
    }

    pub(super) fn on_file_op_completed(
        &mut self,
        action: String,
        result: Result<(), String>,
        refresh_dir: Option<String>,
    ) {
        self.state.is_busy = false;
        match result {
            Ok(()) => {
                self.state.file_transfer_message.clear();
                self.state.file_transfer_percent = -1.0;
                self.state.status_message = format!("{}成功", action);
                if let Some(dir) = refresh_dir {
                    let _ = self.intent_sender.send(AppIntent::BrowseDirectory(dir));
                }
            }
            Err(error) => {
                self.state.file_transfer_message = format!("{}失败: {}", action, error);
                self.state.status_message = format!("{}失败: {}", action, error);
            }
        }
    }

    /// 取消进行中的传输（未在传输时静默忽略）
    pub(super) fn on_cancel_transfer(&mut self) {
        if let Some(cancel) = &self.file_transfer_cancel {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.state.file_transfer_message = "正在取消...".into();
        }
    }

    /// 传输任务结束后的句柄回收，恢复"可发起新传输"状态
    pub(super) fn on_transfer_settled(&mut self) {
        self.file_transfer_cancel = None;
    }

    fn begin_transfer(&mut self) -> Arc<std::sync::atomic::AtomicBool> {
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.file_transfer_cancel = Some(Arc::clone(&cancel));
        cancel
    }

    fn reject_concurrent_transfer(&mut self) {
        self.state.file_transfer_message = "已有传输进行中，可先取消当前传输".into();
    }
}

async fn upload_local_paths(
    usecases: &ikaros_usecase::FileUseCases,
    serial: &str,
    local_paths: &[PathBuf],
    remote_directory: &str,
    tx: &tokio::sync::mpsc::UnboundedSender<AppIntent>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> String {
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let upload = usecases.push_paths(serial, local_paths, remote_directory, Some(progress_tx));
    let cancelled = wait_for_cancel(cancel);
    tokio::pin!(upload, cancelled);
    loop {
        tokio::select! {
            biased;
            _ = &mut cancelled => return format!("已取消上传: {}", remote_directory),
            Some(progress) = progress_rx.recv() => {
                let _ = tx.send(AppIntent::TransferProgressUpdated(progress));
            }
            result = &mut upload => {
                return match result {
                    Ok(()) => format!("上传成功: {}", remote_directory),
                    Err(error) => {
                        error!("上传失败: {}", error);
                        format!("上传失败: {}", error)
                    }
                };
            }
        }
    }
}

/// 轮询取消标志；取消是低频操作，100ms 轮询足够且无需额外依赖
async fn wait_for_cancel(cancel: Arc<std::sync::atomic::AtomicBool>) {
    while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
