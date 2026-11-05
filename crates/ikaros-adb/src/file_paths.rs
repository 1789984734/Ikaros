use crate::{shell_quote, AdbClient};
use ikaros_domain::{DeviceRepository, DomainError};
use std::time::Duration;
use tokio::time::timeout;

const DIRECTORY_OK: &str = "__IKAROS_DIRECTORY_OK__";

#[cfg(test)]
#[path = "file_paths_tests.rs"]
mod tests;

impl AdbClient {
    /// Legacy Sync LIST 对不存在的目录也可能返回空列表，因此先验证目录可读。
    pub(crate) async fn ensure_directory(
        &self,
        serial: &str,
        path: &str,
    ) -> Result<(), DomainError> {
        let path_arg = shell_quote(path);
        let command = format!(
            "if [ -d {path_arg} ] && [ -r {path_arg} ] && [ -x {path_arg} ]; then printf '{DIRECTORY_OK}'; fi"
        );
        let output = timeout(Duration::from_secs(3), self.execute_shell(serial, &command))
            .await
            .map_err(|_| DomainError::FileSyncFailed("验证文件夹超时".into()))??;
        if output.trim() != DIRECTORY_OK {
            return Err(DomainError::FileSyncFailed(format!(
                "目录不存在或无权访问: {path}"
            )));
        }
        Ok(())
    }

    /// 在设备端筛选目录，包含隐藏目录与指向目录的符号链接。
    pub(crate) async fn read_subdirectories(
        &self,
        serial: &str,
        path: &str,
    ) -> Result<Vec<String>, DomainError> {
        self.ensure_directory(serial, path).await?;
        let command = subdirectories_command(path);
        let output = self.execute_shell(serial, &command).await?;
        Ok(output
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

fn subdirectories_command(path: &str) -> String {
    let base = shell_quote(&format!("{}/", path.trim_end_matches('/')));
    format!(
        "for ikaros_path in {base}* {base}.[!.]* {base}..?*; do if [ -d \"$ikaros_path\" ]; then printf '%s\\0' \"$ikaros_path\"; fi; done"
    )
}
