use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use ikaros_domain::{AppFilterType, AppInfo, DeviceRepository, DomainError, SyncRepository};
use serde::Deserialize;

use crate::AdbClient;

const HELPER_DEX: &[u8] = include_bytes!("../device-helper/package-metadata.dex");

#[derive(Deserialize)]
struct DevicePackage {
    #[serde(flatten)]
    app: AppInfo,
    icon_png: String,
}

impl AdbClient {
    pub(crate) async fn read_package_metadata(
        &self,
        serial: &str,
        filter: AppFilterType,
    ) -> Result<Vec<AppInfo>, DomainError> {
        let mut helper = tempfile::NamedTempFile::new().map_err(metadata_io_error)?;
        helper.write_all(HELPER_DEX).map_err(metadata_io_error)?;
        let filename = helper.path().file_name().unwrap().to_string_lossy();
        let remote = format!(
            "/data/local/tmp/ikaros-packages-{}-{}.dex",
            std::process::id(),
            filename
        );
        let deployed = self.push_file(serial, helper.path(), &remote, None).await;
        let result = match deployed {
            Ok(()) => self.run_package_metadata(serial, &remote, filter).await,
            Err(error) => Err(error),
        };
        let _ = self
            .execute_shell(serial, &format!("rm -f '{remote}'"))
            .await;
        let packages = parse_metadata_output(&result?)?;
        tokio::task::spawn_blocking(move || cache_package_icons(packages))
            .await
            .map_err(|error| DomainError::Other(format!("处理应用图标失败: {error}")))
    }

    async fn run_package_metadata(
        &self,
        serial: &str,
        remote: &str,
        filter: AppFilterType,
    ) -> Result<String, DomainError> {
        let filter = match filter {
            AppFilterType::ThirdParty => "third-party",
            AppFilterType::System => "system",
            AppFilterType::All => "all",
        };
        let command = format!(
            "chmod 444 '{remote}' && CLASSPATH='{remote}' app_process /system/bin dev.ikaros.PackageMetadata {filter}"
        );
        tokio::time::timeout(
            Duration::from_secs(60),
            self.execute_shell(serial, &command),
        )
        .await
        .map_err(|_| DomainError::ShellExecutionFailed("读取应用信息超时".into()))?
    }
}

fn parse_metadata_output(output: &str) -> Result<Vec<DevicePackage>, DomainError> {
    let mut packages = Vec::new();
    let mut count = None;
    for line in output.lines().map(str::trim) {
        if let Some(json) = line.strip_prefix("IKAROS_APP:") {
            let package = serde_json::from_str(json)
                .map_err(|error| DomainError::ParseError(format!("应用信息格式错误: {error}")))?;
            packages.push(package);
        } else if let Some(value) = line.strip_prefix("IKAROS_DONE:") {
            count = value.parse::<usize>().ok();
        } else if let Some(error) = line.strip_prefix("IKAROS_ERROR:") {
            return Err(DomainError::ShellExecutionFailed(error.to_string()));
        }
    }
    if count != Some(packages.len()) {
        return Err(DomainError::ParseError("设备返回的应用信息不完整".into()));
    }
    Ok(packages)
}

fn cache_package_icons(packages: Vec<DevicePackage>) -> Vec<AppInfo> {
    let directory = std::env::temp_dir().join("ikaros-app-icons");
    let cache_ready = std::fs::create_dir_all(&directory).is_ok();
    packages
        .into_iter()
        .map(|mut package| {
            if package.app.app_name.trim().is_empty() {
                package.app.app_name = package.app.package_name.clone();
            }
            package.app.icon_path = if cache_ready && !package.icon_png.is_empty() {
                match cache_icon(&directory, &package.icon_png) {
                    Ok(path) => Some(path),
                    Err(error) => {
                        tracing::warn!("缓存 {} 的图标失败: {}", package.app.package_name, error);
                        None
                    }
                }
            } else {
                None
            };
            package.app
        })
        .collect()
}

fn cache_icon(directory: &Path, encoded: &str) -> Result<PathBuf, DomainError> {
    let png = STANDARD
        .decode(encoded)
        .map_err(|error| DomainError::ParseError(format!("图标编码错误: {error}")))?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(DomainError::ParseError("图标不是 PNG 图片".into()));
    }
    // Immutable content keys keep Slint's image cache valid across devices and app updates.
    let mut hash = DefaultHasher::new();
    png.hash(&mut hash);
    let path = directory.join(format!("{:016x}.png", hash.finish()));
    if !path.is_file() {
        let mut file = tempfile::NamedTempFile::new_in(directory).map_err(metadata_io_error)?;
        file.write_all(&png).map_err(metadata_io_error)?;
        if let Err(error) = file.persist_noclobber(&path) {
            if error.error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(metadata_io_error(error.error));
            }
        }
    }
    Ok(path)
}

fn metadata_io_error(error: std::io::Error) -> DomainError {
    DomainError::Other(format!("读写应用资源失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD: &str = r#"IKAROS_APP:{"app_name":"设置","package_name":"com.android.settings","version_name":"15","version_code":"4294967297","apk_path":"/system/Settings.apk","is_system":true,"is_enabled":true,"icon_png":""}"#;

    #[test]
    fn reads_localized_names_and_long_version_codes_amid_runtime_logs() {
        let output = format!("Runtime diagnostic\r\n{RECORD}\r\nIKAROS_DONE:1\r\n");
        let apps = cache_package_icons(parse_metadata_output(&output).unwrap());
        assert_eq!(apps[0].app_name, "设置");
        assert_eq!(apps[0].version_code, "4294967297");
        assert!(apps[0].icon_path.is_none());
    }

    #[test]
    fn rejects_partial_results_instead_of_showing_an_incomplete_list() {
        assert!(parse_metadata_output(RECORD).is_err());
        assert!(parse_metadata_output(&format!("{RECORD}\nIKAROS_DONE:2")).is_err());
        assert!(parse_metadata_output("IKAROS_ERROR:Permission denied").is_err());
        assert!(parse_metadata_output("IKAROS_DONE:0").unwrap().is_empty());
    }

    #[test]
    fn rejects_invalid_icon_data() {
        let directory = tempfile::tempdir().unwrap();
        assert!(cache_icon(directory.path(), "not base64!").is_err());
        assert!(cache_icon(directory.path(), &STANDARD.encode(b"not a PNG")).is_err());
    }
}
