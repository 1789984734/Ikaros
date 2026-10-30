use crate::AdbClient;
use async_trait::async_trait;
use ikaros_domain::{
    AppFilterType, AppInfo, DeviceRepository, DomainError, PackageRepository, SyncRepository,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

pub(crate) fn valid_package_name(package: &str) -> bool {
    !package.is_empty()
        && package.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
        })
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;

#[async_trait]
impl PackageRepository for AdbClient {
    /// 查询已安装应用列表
    async fn list_packages(
        &self,
        serial: &str,
        filter: AppFilterType,
    ) -> Result<Vec<AppInfo>, DomainError> {
        match self.read_package_metadata(serial, filter).await {
            Ok(apps) => return Ok(apps),
            Err(error) => tracing::warn!("读取应用名称和图标失败，回退到基础包列表: {}", error),
        }

        let flag = match filter {
            AppFilterType::ThirdParty => "-3 -f",
            AppFilterType::System => "-s -f",
            AppFilterType::All => "-f",
        };

        let user_id = self.current_user_id(serial).await?;
        let cmd = format!("pm list packages --user {user_id} {flag}");
        let output = self.execute_shell(serial, &cmd).await?;
        let system_packages: HashSet<String> = if filter == AppFilterType::All {
            self.execute_shell(serial, &format!("pm list packages --user {user_id} -s"))
                .await?
                .lines()
                .filter_map(|line| line.trim().strip_prefix("package:").map(str::to_owned))
                .collect()
        } else {
            HashSet::new()
        };

        // 单次全量拉取所有包的版本信息，避免逐包 dumpsys 导致列表加载过慢
        let versions = self.query_all_versions(serial).await?;

        let mut apps = Vec::new();

        for line in output.lines() {
            let line = line.trim();
            // 典型输出格式: package:/data/app/~~.../base.apk=com.tencent.mm
            if let Some(content) = line.strip_prefix("package:") {
                if let Some((apk_path, package_name)) = content.rsplit_once('=') {
                    let is_system =
                        filter == AppFilterType::System || system_packages.contains(package_name);
                    let (version_name, version_code) = versions
                        .get(package_name)
                        .cloned()
                        .unwrap_or_else(|| ("未知".to_string(), "未知".to_string()));

                    apps.push(AppInfo {
                        app_name: package_name.to_string(),
                        package_name: package_name.to_string(),
                        icon_path: None,
                        version_name,
                        version_code,
                        apk_path: apk_path.to_string(),
                        is_system,
                        is_enabled: true,
                    });
                }
            }
        }

        Ok(apps)
    }

    /// 查询指定包名的版本详情
    async fn get_package_detail(
        &self,
        serial: &str,
        package_name: &str,
    ) -> Result<(String, String), DomainError> {
        let cmd = format!("dumpsys package {}", package_name);
        let output = self.execute_shell(serial, &cmd).await?;

        let mut version_name = "未知".to_string();
        let mut version_code = "未知".to_string();

        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("versionName=") {
                version_name = trimmed["versionName=".len()..].to_string();
            } else if trimmed.starts_with("versionCode=") {
                if let Some((code, _)) = trimmed["versionCode=".len()..].split_once(' ') {
                    version_code = code.to_string();
                } else {
                    version_code = trimmed["versionCode=".len()..].to_string();
                }
            }
        }

        Ok((version_name, version_code))
    }

    /// 安装本地 APK 文件
    async fn install_apk(&self, serial: &str, local_apk_path: &Path) -> Result<(), DomainError> {
        let remote_tmp = format!(
            "/data/local/tmp/ikaros_install_{}.apk",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        // 1. 利用 ADB Sync 极速推送 APK 到手机临时目录
        self.push_file(serial, local_apk_path, &remote_tmp, None)
            .await?;

        // 2. 执行安装命令 (覆盖安装 -r，并支持测试包 -t)
        let install_cmd = format!("pm install -r -t \"{}\"", remote_tmp);
        let output = self.execute_shell(serial, &install_cmd).await?;

        // 3. 清理临时 APK
        let _ = self
            .execute_shell(serial, &format!("rm -f \"{}\"", remote_tmp))
            .await;

        if output.contains("Success") {
            Ok(())
        } else {
            Err(DomainError::ShellExecutionFailed(format!(
                "APK 安装失败: {}",
                output.trim()
            )))
        }
    }

    /// 卸载应用，系统应用由用例限制为仅当前用户。
    async fn uninstall_package(
        &self,
        serial: &str,
        package_name: &str,
        current_user_only: bool,
    ) -> Result<(), DomainError> {
        if !valid_package_name(package_name) {
            return Err(DomainError::Other("无效的应用包名".into()));
        }
        let user_option = if current_user_only {
            format!(" --user {}", self.current_user_id(serial).await?)
        } else {
            String::new()
        };
        let cmd = format!("pm uninstall{user_option} {package_name}");
        let output =
            tokio::time::timeout(Duration::from_secs(60), self.execute_shell(serial, &cmd))
                .await
                .map_err(|_| DomainError::ShellExecutionFailed("应用卸载请求超时".into()))??;

        if output.trim() == "Success" {
            Ok(())
        } else {
            Err(DomainError::ShellExecutionFailed(format!(
                "应用卸载失败: {}",
                output.trim()
            )))
        }
    }

    /// 清除应用数据 (pm clear)
    async fn clear_package_data(
        &self,
        serial: &str,
        package_name: &str,
    ) -> Result<(), DomainError> {
        let cmd = format!("pm clear {}", package_name);
        let output = self.execute_shell(serial, &cmd).await?;

        if output.contains("Success") {
            Ok(())
        } else {
            Err(DomainError::ShellExecutionFailed(format!(
                "清除数据失败: {}",
                output.trim()
            )))
        }
    }

    /// 启动应用 (Launcher Activity)
    async fn launch_package(&self, serial: &str, package_name: &str) -> Result<(), DomainError> {
        self.start_package(serial, package_name).await
    }

    /// 强制停止应用
    async fn stop_package(&self, serial: &str, package_name: &str) -> Result<(), DomainError> {
        let cmd = format!("am force-stop {}", package_name);
        let _ = self.execute_shell(serial, &cmd).await?;
        Ok(())
    }
}

impl AdbClient {
    async fn current_user_id(&self, serial: &str) -> Result<u32, DomainError> {
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            self.execute_shell(serial, "am get-current-user"),
        )
        .await
        .map_err(|_| DomainError::ShellExecutionFailed("读取设备当前用户超时".into()))??;
        output.trim().parse::<u32>().map_err(|_| {
            DomainError::ParseError(format!("无法读取设备当前用户: {}", output.trim()))
        })
    }

    /// 解析 dumpsys package 全量输出，构建 包名 -> (versionName, versionCode) 映射
    async fn query_all_versions(
        &self,
        serial: &str,
    ) -> Result<HashMap<String, (String, String)>, DomainError> {
        let output = self.execute_shell(serial, "dumpsys package").await?;

        let mut map = HashMap::new();
        let mut current_pkg: Option<String> = None;
        let mut current_name = String::new();
        let mut current_code = String::new();

        for line in output.lines() {
            let trimmed = line.trim();
            // 包块起始行形如: Package [com.example.app] (1a2b3c4d):
            if let Some(rest) = trimmed.strip_prefix("Package [") {
                if let Some(pkg) = current_pkg.take() {
                    map.insert(pkg, (current_name.clone(), current_code.clone()));
                }
                if let Some(end) = rest.find(']') {
                    current_pkg = Some(rest[..end].to_string());
                    current_name.clear();
                    current_code.clear();
                }
            } else if current_pkg.is_some() {
                // versionCode 行可能附带 minSdk/targetSdk 等后缀，仅取第一段数字
                if let Some(v) = trimmed.strip_prefix("versionName=") {
                    current_name = v.trim().to_string();
                } else if let Some(v) = trimmed.strip_prefix("versionCode=") {
                    current_code = v.split_whitespace().next().unwrap_or(v).to_string();
                }
            }
        }
        if let Some(pkg) = current_pkg {
            map.insert(pkg, (current_name, current_code));
        }

        Ok(map)
    }
}
