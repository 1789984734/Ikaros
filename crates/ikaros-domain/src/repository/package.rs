use crate::entity::{AppFilterType, AppInfo};
use crate::error::DomainError;
use async_trait::async_trait;
use std::path::Path;

/// 应用包管理领域仓储契约
#[async_trait]
pub trait PackageRepository: Send + Sync {
    /// 获取设备上安装的应用包列表
    async fn list_packages(
        &self,
        serial: &str,
        filter: AppFilterType,
    ) -> Result<Vec<AppInfo>, DomainError>;

    /// 查询单个包的详细版本信息
    async fn get_package_detail(
        &self,
        serial: &str,
        package_name: &str,
    ) -> Result<(String, String), DomainError>;

    /// 安装本地 APK 文件到设备
    async fn install_apk(&self, serial: &str, local_apk_path: &Path) -> Result<(), DomainError>;

    /// 卸载应用；current_user_only 为 true 时只对设备当前用户卸载。
    async fn uninstall_package(
        &self,
        serial: &str,
        package_name: &str,
        current_user_only: bool,
    ) -> Result<(), DomainError>;

    /// 清除应用数据 (pm clear)
    async fn clear_package_data(&self, serial: &str, package_name: &str)
        -> Result<(), DomainError>;

    /// 启动应用 (Launcher Activity)
    async fn launch_package(&self, serial: &str, package_name: &str) -> Result<(), DomainError>;

    /// 强行停止应用进程 (am force-stop)
    async fn stop_package(&self, serial: &str, package_name: &str) -> Result<(), DomainError>;
}
