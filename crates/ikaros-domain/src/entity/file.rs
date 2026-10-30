use serde::{Deserialize, Serialize};

/// 远程文件或目录属性
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteFile {
    /// 文件或目录名
    pub name: String,
    /// 完整绝对路径
    pub path: String,
    /// 文件大小（字节）
    pub size: u64,
    /// 是否为目录
    pub is_dir: bool,
    /// 是否为软链接
    pub is_symlink: bool,
    /// Linux 权限位描述（如 "drwxr-xr-x"）
    pub permissions: String,
    /// 最后修改时间戳（秒）
    pub modified_time: u64,
}

/// 文件传输进度更新
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgress {
    /// 当前传输的文件名
    pub filename: String,
    /// 已传输字节数
    pub transferred_bytes: u64,
    /// 总字节数
    pub total_bytes: u64,
    /// 传输速度（字节/秒）
    pub speed_bytes_per_sec: u64,
    /// 是否已完成
    pub is_completed: bool,
}
