//! 投屏自定义参数 Tag 的本地持久化 (JSON 文件)

use std::fs;
use std::path::{Path, PathBuf};

/// Tag 持久化文件路径: macOS 存 Application Support, 其余平台存各自用户配置目录
fn config_file_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Ikaros"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Ikaros"))
    } else {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/ikaros"))
    }?;
    Some(base.join("scrcpy_tags.json"))
}

/// 启动时读取已保存的自定义参数, 文件缺失或损坏视为空列表
pub fn load_tags() -> Vec<String> {
    let Some(path) = config_file_path() else {
        return Vec::new();
    };
    read_tags(&path)
}

/// 增删 Tag 后整体覆盖保存, 失败仅记录告警不阻塞界面
pub fn save_tags(tags: &[String]) {
    let Some(path) = config_file_path() else {
        return;
    };
    if let Err(error) = write_tags(&path, tags) {
        tracing::warn!(%error, "保存自定义参数 Tag 失败");
    }
}

fn read_tags(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_tags(path: &Path, tags: &[String]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(tags).map_err(|error| error.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    fs::write(path, json).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_roundtrip_through_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scrcpy_tags.json");
        assert!(read_tags(&path).is_empty());

        let tags = vec![
            "--turn-screen-off".to_string(),
            "--record=a b.mp4".to_string(),
        ];
        write_tags(&path, &tags).unwrap();
        assert_eq!(read_tags(&path), tags);
    }

    #[test]
    fn corrupted_tag_file_is_treated_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scrcpy_tags.json");
        fs::write(&path, "not json").unwrap();
        assert!(read_tags(&path).is_empty());
    }
}
