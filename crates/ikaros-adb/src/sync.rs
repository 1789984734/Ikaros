use crate::{shell_quote, AdbClient};
use async_trait::async_trait;
use ikaros_domain::{DeviceRepository, DomainError, RemoteFile, SyncRepository, TransferProgress};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 进度事件的发送间隔; 大文件按块回调非常频繁, 直接透传会以每块一次的频率刷爆 UI 刷新通道
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(100);

/// 传输进度节流器: 首次与距上次发送超过间隔时才放行
struct ProgressThrottle {
    interval: Duration,
    last_emit: Option<Instant>,
}

impl ProgressThrottle {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_emit: None,
        }
    }

    fn should_emit(&mut self, now: Instant) -> bool {
        let allowed = self
            .last_emit
            .map_or(true, |t| now.duration_since(t) >= self.interval);
        if allowed {
            self.last_emit = Some(now);
        }
        allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_emits_first_then_only_after_interval() {
        let mut throttle = ProgressThrottle::new(Duration::from_millis(50));
        assert!(throttle.should_emit(Instant::now()));
        assert!(!throttle.should_emit(Instant::now()));
        std::thread::sleep(Duration::from_millis(60));
        assert!(throttle.should_emit(Instant::now()));
    }
}

// S_IFDIR = 0o040000 (目录标志)
const S_IFDIR: u32 = 0o040000;
// S_IFLNK = 0o120000 (符号链接标志)
const S_IFLNK: u32 = 0o120000;

impl AdbClient {
    /// 执行一条会回显 OK 标记的远程命令，未见到标记即视为失败并携带设备端报错
    async fn run_file_shell(
        &self,
        serial: &str,
        command: &str,
        action: &str,
    ) -> Result<(), DomainError> {
        let output = self
            .execute_shell(serial, &format!("{} && echo __IKAROS_OK__", command))
            .await?;
        if output.contains("__IKAROS_OK__") {
            Ok(())
        } else {
            Err(DomainError::FileSyncFailed(format!(
                "{}失败: {}",
                action,
                output.trim().trim_end_matches('\r')
            )))
        }
    }

    /// 建立并握手进入 ADB Sync 协议二进制模式
    async fn open_sync_connection(&self, serial: &str) -> Result<TcpStream, DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, &format!("host:transport:{}", serial)).await?;
        Self::write_request(&mut stream, "sync:").await?;
        Ok(stream)
    }

    /// 发送一个 4 字节 ID + 4 字节小端序长度 + 数据内容的 Sync 命令
    async fn send_sync_request(
        stream: &mut TcpStream,
        id: &[u8; 4],
        payload: &[u8],
    ) -> Result<(), DomainError> {
        stream.write_all(id).await.map_err(|e| {
            DomainError::FileSyncFailed(format!("发送 Sync 命令 {:?} 失败: {}", id, e))
        })?;
        let len_le = (payload.len() as u32).to_le_bytes();
        stream
            .write_all(&len_le)
            .await
            .map_err(|e| DomainError::FileSyncFailed(format!("发送长度前缀失败: {}", e)))?;
        stream
            .write_all(payload)
            .await
            .map_err(|e| DomainError::FileSyncFailed(format!("发送数据载荷失败: {}", e)))?;
        Ok(())
    }

    /// 优雅退出 Sync 会话
    async fn quit_sync(mut stream: TcpStream) {
        let _ = stream.write_all(b"QUIT\0\0\0\0").await;
    }

    /// STAT 查询远程文件大小; 设备拒绝或响应异常时返回 None（进度条退化为只显示已传字节数）
    async fn stat_remote_size(
        stream: &mut TcpStream,
        remote_path: &str,
    ) -> Result<Option<u64>, DomainError> {
        Self::send_sync_request(stream, b"STAT", remote_path.as_bytes()).await?;

        let mut tag = [0u8; 4];
        stream
            .read_exact(&mut tag)
            .await
            .map_err(|e| DomainError::FileSyncFailed(format!("读取 STAT 响应标志失败: {}", e)))?;

        if &tag == b"STAT" {
            let mut meta = [0u8; 12];
            stream
                .read_exact(&mut meta)
                .await
                .map_err(|e| DomainError::FileSyncFailed(format!("读取 STAT 元数据失败: {}", e)))?;
            return Ok(Some(
                u32::from_le_bytes(meta[4..8].try_into().unwrap()) as u64
            ));
        }

        if &tag == b"FAIL" {
            // 消费错误消息, 连接仍可继续用于后续 RECV
            let mut len_buf = [0u8; 4];
            let _ = stream.read_exact(&mut len_buf).await;
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut msg = vec![0u8; len];
            let _ = stream.read_exact(&mut msg).await;
            return Ok(None);
        }

        Err(DomainError::FileSyncFailed(format!(
            "未知的 Sync STAT 响应标记: {:?}",
            String::from_utf8_lossy(&tag)
        )))
    }
}

#[async_trait]
impl SyncRepository for AdbClient {
    /// 列出远程目录文件列表 (LIST 协议)
    async fn list_directory(
        &self,
        serial: &str,
        path: &str,
    ) -> Result<Vec<RemoteFile>, DomainError> {
        let mut stream = self.open_sync_connection(serial).await?;
        Self::send_sync_request(&mut stream, b"LIST", path.as_bytes()).await?;

        let mut files = Vec::new();

        loop {
            let mut tag = [0u8; 4];
            stream.read_exact(&mut tag).await.map_err(|e| {
                DomainError::FileSyncFailed(format!("读取 LIST 响应标志失败: {}", e))
            })?;

            match &tag {
                b"DENT" => {
                    let mut meta = [0u8; 16];
                    stream.read_exact(&mut meta).await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("读取条目元数据失败: {}", e))
                    })?;

                    let mode = u32::from_le_bytes(meta[0..4].try_into().unwrap());
                    let size = u32::from_le_bytes(meta[4..8].try_into().unwrap()) as u64;
                    let mtime = u32::from_le_bytes(meta[8..12].try_into().unwrap()) as u64;
                    let namelen = u32::from_le_bytes(meta[12..16].try_into().unwrap()) as usize;

                    let mut name_buf = vec![0u8; namelen];
                    stream.read_exact(&mut name_buf).await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("读取文件名失败: {}", e))
                    })?;

                    let name = String::from_utf8_lossy(&name_buf).to_string();

                    // 过滤当前目录和上级目录标记
                    if name == "." || name == ".." {
                        continue;
                    }

                    let is_dir = (mode & S_IFDIR) != 0;
                    let is_symlink = (mode & S_IFLNK) != 0;

                    let full_path = if path.ends_with('/') {
                        format!("{}{}", path, name)
                    } else {
                        format!("{}/{}", path, name)
                    };

                    files.push(RemoteFile {
                        name,
                        path: full_path,
                        size,
                        is_dir,
                        is_symlink,
                        permissions: format!("{:o}", mode & 0o777),
                        modified_time: mtime,
                    });
                }
                b"DONE" => {
                    // 读取结尾保留的 16 字节
                    let mut done_pad = [0u8; 16];
                    let _ = stream.read_exact(&mut done_pad).await;
                    Self::quit_sync(stream).await;
                    break;
                }
                other => {
                    return Err(DomainError::FileSyncFailed(format!(
                        "未知的 Sync LIST 响应标记: {:?}",
                        String::from_utf8_lossy(other)
                    )));
                }
            }
        }

        Ok(files)
    }

    /// 从手机拉取文件到本地 (RECV 协议)
    async fn pull_file(
        &self,
        serial: &str,
        remote_path: &str,
        local_path: &Path,
        progress_tx: Option<tokio::sync::mpsc::UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        let mut stream = self.open_sync_connection(serial).await?;
        // 预先 STAT 获取真实大小, 让下载进度条可以显示真实百分比
        let total_size = Self::stat_remote_size(&mut stream, remote_path)
            .await?
            .unwrap_or(0);
        Self::send_sync_request(&mut stream, b"RECV", remote_path.as_bytes()).await?;

        // 创建本地目标文件
        let mut local_file = File::create(local_path).await.map_err(|e| {
            DomainError::FileSyncFailed(format!("创建本地文件 ({:?}) 失败: {}", local_path, e))
        })?;

        let filename = Path::new(remote_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(remote_path)
            .to_string();

        let mut transferred = 0u64;
        let start_time = Instant::now();
        let mut throttle = ProgressThrottle::new(PROGRESS_EMIT_INTERVAL);

        loop {
            let mut tag = [0u8; 4];
            stream.read_exact(&mut tag).await.map_err(|e| {
                DomainError::FileSyncFailed(format!("读取 RECV 响应标志失败: {}", e))
            })?;

            match &tag {
                b"DATA" => {
                    let mut len_buf = [0u8; 4];
                    stream.read_exact(&mut len_buf).await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("读取数据块长度失败: {}", e))
                    })?;
                    let chunk_len = u32::from_le_bytes(len_buf) as usize;

                    let mut chunk = vec![0u8; chunk_len];
                    stream.read_exact(&mut chunk).await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("读取数据块内容失败: {}", e))
                    })?;

                    local_file.write_all(&chunk).await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("写入本地文件失败: {}", e))
                    })?;

                    transferred += chunk_len as u64;
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        (transferred as f64 / elapsed) as u64
                    } else {
                        0
                    };

                    if let Some(tx) = &progress_tx {
                        if throttle.should_emit(Instant::now()) {
                            let _ = tx.send(TransferProgress {
                                filename: filename.clone(),
                                transferred_bytes: transferred,
                                total_bytes: total_size,
                                speed_bytes_per_sec: speed,
                                is_completed: false,
                            });
                        }
                    }
                }
                b"DONE" => {
                    let mut _pad = [0u8; 4];
                    let _ = stream.read_exact(&mut _pad).await;
                    local_file.flush().await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("刷新本地文件缓冲区失败: {}", e))
                    })?;

                    if let Some(tx) = &progress_tx {
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 {
                            (transferred as f64 / elapsed) as u64
                        } else {
                            0
                        };
                        let _ = tx.send(TransferProgress {
                            filename,
                            transferred_bytes: transferred,
                            total_bytes: transferred,
                            speed_bytes_per_sec: speed,
                            is_completed: true,
                        });
                    }

                    Self::quit_sync(stream).await;
                    break;
                }
                b"FAIL" => {
                    let mut len_buf = [0u8; 4];
                    stream.read_exact(&mut len_buf).await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("读取错误长度失败: {}", e))
                    })?;
                    let err_len = u32::from_le_bytes(len_buf) as usize;
                    let mut err_msg = vec![0u8; err_len];
                    stream.read_exact(&mut err_msg).await.map_err(|e| {
                        DomainError::FileSyncFailed(format!("读取错误信息失败: {}", e))
                    })?;
                    return Err(DomainError::FileSyncFailed(format!(
                        "拉取文件失败: {}",
                        String::from_utf8_lossy(&err_msg)
                    )));
                }
                other => {
                    if other == b"FAIL" {
                        let mut len_buf = [0u8; 4];
                        stream.read_exact(&mut len_buf).await.map_err(|e| {
                            DomainError::FileSyncFailed(format!("读取失败信息长度失败: {}", e))
                        })?;
                        let len = u32::from_le_bytes(len_buf) as usize;
                        let mut msg = vec![0u8; len];
                        stream.read_exact(&mut msg).await.map_err(|e| {
                            DomainError::FileSyncFailed(format!("读取失败信息失败: {}", e))
                        })?;
                        return Err(DomainError::FileSyncFailed(format!(
                            "拉取文件失败: {}",
                            String::from_utf8_lossy(&msg)
                        )));
                    }
                    return Err(DomainError::FileSyncFailed(format!(
                        "未知的 Sync RECV 响应标记: {:?}",
                        String::from_utf8_lossy(other)
                    )));
                }
            }
        }

        Ok(())
    }

    /// 从本地推送文件到手机 (SEND 协议)
    async fn push_file(
        &self,
        serial: &str,
        local_path: &Path,
        remote_path: &str,
        progress_tx: Option<tokio::sync::mpsc::UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        let mut stream = self.open_sync_connection(serial).await?;

        // 默认权限 0644 (33188)
        let header = format!("{},33188", remote_path);
        Self::send_sync_request(&mut stream, b"SEND", header.as_bytes()).await?;

        let mut local_file = File::open(local_path).await.map_err(|e| {
            DomainError::FileSyncFailed(format!(
                "打开本地待推送文件 ({:?}) 失败: {}",
                local_path, e
            ))
        })?;

        let total_size = local_file.metadata().await.map(|m| m.len()).unwrap_or(0);

        let filename = local_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("file")
            .to_string();

        let mut transferred = 0u64;
        let start_time = Instant::now();
        let mut throttle = ProgressThrottle::new(PROGRESS_EMIT_INTERVAL);
        // 64KB Chunk
        let mut buf = vec![0u8; 64 * 1024];

        loop {
            let n = local_file
                .read(&mut buf)
                .await
                .map_err(|e| DomainError::FileSyncFailed(format!("读取本地文件失败: {}", e)))?;
            if n == 0 {
                break;
            }

            // 发送 DATA 包
            stream
                .write_all(b"DATA")
                .await
                .map_err(|e| DomainError::FileSyncFailed(format!("发送 DATA 标记失败: {}", e)))?;
            let len_le = (n as u32).to_le_bytes();
            stream
                .write_all(&len_le)
                .await
                .map_err(|e| DomainError::FileSyncFailed(format!("发送 DATA 块长度失败: {}", e)))?;
            stream
                .write_all(&buf[..n])
                .await
                .map_err(|e| DomainError::FileSyncFailed(format!("发送 DATA 块内容失败: {}", e)))?;

            transferred += n as u64;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                (transferred as f64 / elapsed) as u64
            } else {
                0
            };

            if let Some(tx) = &progress_tx {
                if throttle.should_emit(Instant::now()) {
                    let _ = tx.send(TransferProgress {
                        filename: filename.clone(),
                        transferred_bytes: transferred,
                        total_bytes: total_size,
                        speed_bytes_per_sec: speed,
                        is_completed: false,
                    });
                }
            }
        }

        // 发送 DONE 包携带当前时间戳
        let now_sec = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;
        stream
            .write_all(b"DONE")
            .await
            .map_err(|e| DomainError::FileSyncFailed(format!("发送 DONE 标记失败: {}", e)))?;
        stream
            .write_all(&now_sec.to_le_bytes())
            .await
            .map_err(|e| DomainError::FileSyncFailed(format!("发送 DONE 时间戳失败: {}", e)))?;

        // 校验设备端确认回复
        let mut status = [0u8; 4];
        stream
            .read_exact(&mut status)
            .await
            .map_err(|e| DomainError::FileSyncFailed(format!("读取推送结果响应失败: {}", e)))?;

        if &status != b"OKAY" {
            let mut len_buf = [0u8; 4];
            let _ = stream.read_exact(&mut len_buf).await;
            let err_len = u32::from_le_bytes(len_buf) as usize;
            let mut err_msg = vec![0u8; err_len];
            let _ = stream.read_exact(&mut err_msg).await;
            return Err(DomainError::FileSyncFailed(format!(
                "推送文件失败: {}",
                String::from_utf8_lossy(&err_msg)
            )));
        }

        if let Some(tx) = &progress_tx {
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.0 {
                (transferred as f64 / elapsed) as u64
            } else {
                0
            };
            let _ = tx.send(TransferProgress {
                filename,
                transferred_bytes: transferred,
                total_bytes: total_size,
                speed_bytes_per_sec: speed,
                is_completed: true,
            });
        }

        Self::quit_sync(stream).await;
        Ok(())
    }

    async fn delete_path(&self, serial: &str, remote_path: &str) -> Result<(), DomainError> {
        if remote_path.trim().is_empty() || remote_path == "/" {
            return Err(DomainError::FileSyncFailed("非法的删除路径".into()));
        }
        self.run_file_shell(
            serial,
            &format!("rm -rf {}", shell_quote(remote_path)),
            "删除",
        )
        .await
    }

    async fn rename_path(
        &self,
        serial: &str,
        remote_path: &str,
        new_name: &str,
    ) -> Result<(), DomainError> {
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name.contains('/') {
            return Err(DomainError::FileSyncFailed(
                "新名称不能为空或包含路径分隔符".into(),
            ));
        }
        let parent = match remote_path.trim_end_matches('/').rsplit_once('/') {
            Some((parent, _)) => parent,
            None => return Err(DomainError::FileSyncFailed("非法的重命名路径".into())),
        };
        let target = format!("{}/{}", parent, new_name);
        self.run_file_shell(
            serial,
            &format!("mv {} {}", shell_quote(remote_path), shell_quote(&target)),
            "重命名",
        )
        .await
    }

    async fn make_directory(&self, serial: &str, remote_path: &str) -> Result<(), DomainError> {
        if remote_path.trim().is_empty() {
            return Err(DomainError::FileSyncFailed("目录名不能为空".into()));
        }
        self.run_file_shell(
            serial,
            &format!("mkdir -p {}", shell_quote(remote_path)),
            "创建目录",
        )
        .await
    }
}
