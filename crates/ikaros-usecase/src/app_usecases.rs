use std::path::Path;
use std::sync::Arc;
use ikaros_domain::{AppFilterType, AppInfo, DomainError, PackageRepository};

/// 应用管理业务用例
pub struct AppUseCases {
    package_repo: Arc<dyn PackageRepository>,
}

impl AppUseCases {
    pub fn new(package_repo: Arc<dyn PackageRepository>) -> Self {
        Self { package_repo }
    }

    /// 用例：查询已安装应用列表并排序
    pub async fn get_installed_packages(
        &self,
        serial: &str,
        filter: AppFilterType,
    ) -> Result<Vec<AppInfo>, DomainError> {
        let mut list = self.package_repo.list_packages(serial, filter).await?;
        list.sort_by(|a, b| a.package_name.to_lowercase().cmp(&b.package_name.to_lowercase()));
        Ok(list)
    }

    /// 用例：安装 APK
    pub async fn install_apk(&self, serial: &str, apk_path: &Path) -> Result<(), DomainError> {
        self.package_repo.install_apk(serial, apk_path).await
    }

    /// 用例：卸载应用，系统应用只对设备当前用户卸载。
    pub async fn uninstall_app(&self, serial: &str, app: &AppInfo) -> Result<(), DomainError> {
        self.package_repo
            .uninstall_package(serial, &app.package_name, app.is_system)
            .await
    }

    /// 用例：清除应用数据
    pub async fn clear_app_data(&self, serial: &str, package_name: &str) -> Result<(), DomainError> {
        self.package_repo.clear_package_data(serial, package_name).await
    }

    /// 用例：启动应用
    pub async fn launch_app(&self, serial: &str, package_name: &str) -> Result<(), DomainError> {
        self.package_repo.launch_package(serial, package_name).await
    }

    /// 用例：强制停止应用
    pub async fn stop_app(&self, serial: &str, package_name: &str) -> Result<(), DomainError> {
        self.package_repo.stop_package(serial, package_name).await
    }
}
