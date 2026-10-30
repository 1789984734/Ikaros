//! 应用全局配置的本地持久化 (JSON 文件)

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// 外部 scrcpy 本体所在目录（持久化，重启后恢复）
    #[serde(default)]
    pub scrcpy_path: Option<String>,
    /// Last custom arguments; Tag selection is derived from these on startup.
    #[serde(default)]
    pub scrcpy_extra_args: String,
    /// 文件下载默认保存目录（缺省为系统 Downloads）
    #[serde(default)]
    pub default_download_dir: Option<String>,
    /// 无线连接地址历史（最新在前，最多保留 8 条）
    #[serde(default)]
    pub wireless_history: Vec<String>,
}

fn config_file_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Ikaros"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Ikaros"))
    } else {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/ikaros"))
    }?;
    Some(base.join("config.json"))
}

/// 启动时读取配置；文件缺失或损坏时返回默认值
pub fn load() -> AppConfig {
    let Some(path) = config_file_path() else {
        return AppConfig::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// 全量保存配置；失败仅记录日志，不打断交互
pub fn save(config: &AppConfig) {
    let Some(path) = config_file_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            if let Err(error) = fs::write(path, json) {
                tracing::warn!("保存应用配置失败: {error}");
            }
        }
        Err(error) => tracing::warn!("序列化应用配置失败: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_all_fields() {
        let config = AppConfig {
            scrcpy_path: Some("/Applications/scrcpy".into()),
            scrcpy_extra_args: "--turn-screen-off '--record=/tmp/a b.mp4'".into(),
            default_download_dir: Some("/Users/demo/Downloads".into()),
            wireless_history: vec!["192.168.1.8:5555".into()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn older_config_without_custom_arguments_keeps_existing_settings() {
        let parsed: AppConfig = serde_json::from_str(
            r#"{
                "scrcpy_path": "/Applications/scrcpy",
                "default_download_dir": "/Users/demo/Downloads",
                "wireless_history": ["192.168.1.8:5555"]
            }"#,
        )
        .unwrap();
        assert_eq!(
            parsed,
            AppConfig {
                scrcpy_path: Some("/Applications/scrcpy".into()),
                default_download_dir: Some("/Users/demo/Downloads".into()),
                wireless_history: vec!["192.168.1.8:5555".into()],
                ..AppConfig::default()
            }
        );
    }

    #[test]
    fn broken_config_file_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<AppConfig>(&raw).ok()),
            None
        );
    }
}
