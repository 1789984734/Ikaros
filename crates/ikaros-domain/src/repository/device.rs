use async_trait::async_trait;
use crate::entity::{Device, DeviceDetail};
use crate::error::DomainError;
use tokio::sync::{mpsc::UnboundedSender, watch::Receiver};

/// 设备操作领域仓储契约（接口定义）
#[async_trait]
pub trait DeviceRepository: Send + Sync {
    /// 查询当前连接的所有设备
    async fn list_devices(&self) -> Result<Vec<Device>, DomainError>;

    /// 监听设备热插拔与状态变化（设备上线、下线）
    /// 返回一个流或通过回调通道通知
    async fn track_devices(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<Device>>,
    ) -> Result<(), DomainError>;

    /// 读取指定的系统属性（getprop）
    async fn get_property(&self, serial: &str, prop_name: &str) -> Result<String, DomainError>;

    /// 获取设备的完整详情（品牌、型号、版本、电池等）
    async fn get_device_detail(&self, serial: &str) -> Result<DeviceDetail, DomainError>;

    /// 在设备上执行远程 Shell 命令
    async fn execute_shell(&self, serial: &str, command: &str) -> Result<String, DomainError>;

    /// 在设备上执行流式 Shell 命令，实时回传输出文本，收到停止信号或 EOF 时结束
    async fn stream_shell(
        &self,
        serial: &str,
        command: &str,
        chunk_tx: UnboundedSender<String>,
        stop_rx: Receiver<bool>,
    ) -> Result<(), DomainError>;

    /// 执行设备端命令并把二进制输出流式写入本地文件（如截图、bugreport）
    async fn exec_to_file(
        &self,
        serial: &str,
        command: &str,
        local_path: &std::path::Path,
        stop_rx: Receiver<bool>,
    ) -> Result<u64, DomainError>;

    /// 连接一台已开启无线调试的远程设备（host:connect）
    async fn connect_remote(&self, address: &str) -> Result<String, DomainError>;

    /// 断开一台远程无线设备（host:disconnect）
    async fn disconnect_remote(&self, address: &str) -> Result<String, DomainError>;

    /// 让 USB 设备切换到 TCP 无线模式并监听 5555 端口（tcpip）
    async fn enable_tcpip(&self, serial: &str, port: u16) -> Result<String, DomainError>;

    /// 设置端口转发 (adb forward)
    async fn forward_port(
        &self,
        serial: &str,
        local_port: u16,
        remote_abstract: &str,
    ) -> Result<(), DomainError>;

    /// 设置反向端口代理 (adb reverse)
    async fn reverse_port(
        &self,
        serial: &str,
        remote_port: u16,
        local_abstract: &str,
    ) -> Result<(), DomainError>;
}
