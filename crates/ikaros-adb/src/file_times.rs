use crate::{shell_quote, AdbClient};
use ikaros_domain::{DeviceRepository, RemoteFile};
use std::time::Duration;
use tokio::time::timeout;

const CREATION_TIME_TIMEOUT: Duration = Duration::from_secs(2);
const CREATION_TIME_BATCH_SIZE: usize = 32;

#[cfg(test)]
#[path = "file_times_tests.rs"]
mod tests;

impl AdbClient {
    /// 可选元数据：仅使用 stat 的 birth time，失败时保留未知，不影响目录浏览。
    pub(crate) async fn load_creation_times(
        &self,
        serial: &str,
        directory: &str,
        files: &mut [RemoteFile],
    ) {
        if files.is_empty() {
            return;
        }
        let probe = format!("stat -c '%W' -- {} 2>/dev/null", shell_quote(directory));
        let Ok(Ok(output)) =
            timeout(CREATION_TIME_TIMEOUT, self.execute_shell(serial, &probe)).await
        else {
            return;
        };
        // Toybox 不支持 %W 时会输出问号；0 表示语法受支持但该路径无创建时间。
        if output.trim().parse::<i64>().is_err() {
            return;
        }
        for batch in files.chunks_mut(CREATION_TIME_BATCH_SIZE) {
            let paths = batch
                .iter()
                .map(|file| shell_quote(&file.path))
                .collect::<Vec<_>>()
                .join(" ");
            let command = format!(
                "for ikaros_file_path in {paths}; do stat -c '%W' -- \"$ikaros_file_path\" 2>/dev/null || printf '0\\n'; done"
            );
            if let Ok(Ok(output)) =
                timeout(CREATION_TIME_TIMEOUT, self.execute_shell(serial, &command)).await
            {
                apply_creation_times(batch, &output);
            }
        }
    }
}

fn apply_creation_times(files: &mut [RemoteFile], output: &str) {
    let values: Vec<_> = output.lines().collect();
    if values.len() != files.len() {
        return;
    }
    for (file, value) in files.iter_mut().zip(values) {
        file.created_time = value.trim().parse::<u64>().ok().filter(|&time| time > 0);
    }
}
