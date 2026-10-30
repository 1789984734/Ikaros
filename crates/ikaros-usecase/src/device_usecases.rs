use ikaros_domain::{Device, DeviceDetail, DeviceRepository, DomainError};
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, watch::Receiver};

/// 设备管理相关用例集合
pub struct DeviceUseCases {
    repo: Arc<dyn DeviceRepository>,
}

impl DeviceUseCases {
    /// 通过依赖注入构造用例
    pub fn new(repo: Arc<dyn DeviceRepository>) -> Self {
        Self { repo }
    }

    /// 用例：获取当前所有可用连接的设备
    pub async fn get_devices(&self) -> Result<Vec<Device>, DomainError> {
        self.repo.list_devices().await
    }

    /// 用例：查询单台设备的系统软硬件详细信息
    pub async fn get_device_detail(&self, serial: &str) -> Result<DeviceDetail, DomainError> {
        self.repo.get_device_detail(serial).await
    }

    /// 用例：在指定设备上执行 Shell 命令行
    pub async fn execute_shell(&self, serial: &str, command: &str) -> Result<String, DomainError> {
        self.repo.execute_shell(serial, command).await
    }

    /// 用例：流式执行远程 Shell 命令并实时回传输出
    pub async fn stream_shell(
        &self,
        serial: &str,
        command: &str,
        chunk_tx: UnboundedSender<String>,
        stop_rx: Receiver<bool>,
    ) -> Result<(), DomainError> {
        self.repo
            .stream_shell(serial, command, chunk_tx, stop_rx)
            .await
    }

    /// 用例：把设备端命令的二进制输出流式写入本地文件（截图、录屏产物、bugreport）
    pub async fn exec_to_file(
        &self,
        serial: &str,
        command: &str,
        local_path: &std::path::Path,
        stop_rx: Receiver<bool>,
    ) -> Result<u64, DomainError> {
        self.repo
            .exec_to_file(serial, command, local_path, stop_rx)
            .await
    }

    /// 用例：连接一台已开启无线调试的远程设备
    pub async fn connect_remote(&self, address: &str) -> Result<String, DomainError> {
        self.repo.connect_remote(address).await
    }

    /// 用例：断开一台远程无线设备
    pub async fn disconnect_remote(&self, address: &str) -> Result<String, DomainError> {
        self.repo.disconnect_remote(address).await
    }

    /// 用例：让 USB 设备进入 TCP 无线模式
    pub async fn enable_wireless(&self, serial: &str, port: u16) -> Result<String, DomainError> {
        self.repo.enable_tcpip(serial, port).await
    }

    /// 用例：持续监听设备热插拔状态
    pub async fn track_devices(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<Device>>,
    ) -> Result<(), DomainError> {
        self.repo.track_devices(tx).await
    }
}
