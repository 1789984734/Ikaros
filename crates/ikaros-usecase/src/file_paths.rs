use super::FileUseCases;
use ikaros_domain::DomainError;

/// Android 路径使用正斜杠；相对路径以当前目录为起点。
pub fn normalize_remote_path(current: &str, input: &str) -> Result<String, DomainError> {
    if input.trim().is_empty() || input.contains('\0') {
        return Err(DomainError::FileSyncFailed("请输入有效的文件夹路径".into()));
    }
    let path = if input.starts_with('/') {
        input.to_owned()
    } else {
        format!("{}/{input}", current.trim_end_matches('/'))
    };
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

impl FileUseCases {
    /// 查询路径前缀匹配的目录，返回可直接继续输入的绝对路径。
    pub async fn complete_directory_path(
        &self,
        serial: &str,
        current: &str,
        input: &str,
    ) -> Result<Vec<String>, DomainError> {
        let path = normalize_remote_path(current, input)?;
        let (parent, prefix) =
            if input.ends_with('/') || input.ends_with("/.") || input.ends_with("/..") {
                (path.as_str(), "")
            } else {
                let (parent, prefix) = path.rsplit_once('/').unwrap();
                (if parent.is_empty() { "/" } else { parent }, prefix)
            };
        let mut choices = self.sync_repo.list_subdirectories(serial, parent).await?;
        choices.retain(|choice| {
            choice
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with(prefix))
        });
        choices.sort();
        choices.dedup();
        choices.truncate(8);
        Ok(choices
            .into_iter()
            .map(|path| format!("{}/", path.trim_end_matches('/')))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_android_paths_without_losing_spaces_or_unicode() {
        for (input, expected) in [
            ("/", "/"),
            ("//sdcard/./Download/../Music/", "/sdcard/Music"),
            ("../Download", "/sdcard/Download"),
            ("../../../../", "/"),
            ("中文 资料", "/sdcard/Music/中文 资料"),
            ("/sdcard/quote ' $(id)/", "/sdcard/quote ' $(id)"),
        ] {
            assert_eq!(
                normalize_remote_path("/sdcard/Music", input).unwrap(),
                expected
            );
        }
        for input in ["", "   ", "/sdcard/\0"] {
            assert!(normalize_remote_path("/sdcard", input).is_err());
        }
    }
}
