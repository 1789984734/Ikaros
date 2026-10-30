//! 外部官方 scrcpy 客户端的启动参数拼装与进程管理

use std::path::{Path, PathBuf};
use std::process::Stdio;

/// 分辨率上限预设 (0 表示不限制), 下标与 UI 下拉框一一对应
pub const MAX_SIZE_PRESETS: [u16; 6] = [0, 640, 800, 1024, 1280, 1920];
/// 帧率预设 (0 表示使用官方默认)
pub const MAX_FPS_PRESETS: [u16; 5] = [0, 30, 60, 90, 120];
/// 码率预设, UI 下标 0 表示默认, 从 1 开始对应
pub const BIT_RATE_PRESETS: [&str; 4] = ["4M", "8M", "16M", "32M"];
/// 视频编码预设, UI 下标 0 表示默认
pub const VIDEO_CODECS: [&str; 3] = ["h264", "h265", "av1"];

/// 从 UI 收集的 scrcpy 启动配置 (None 表示省略对应命令行参数)
#[derive(Debug, Clone)]
pub struct ScrcpyLaunchOptions {
    /// scrcpy 可执行文件路径或其所在目录
    pub binary_path: String,
    pub max_size: Option<u16>,
    pub max_fps: Option<u16>,
    pub bit_rate: Option<String>,
    pub video_codec: Option<String>,
    pub audio: bool,
    pub stay_awake: bool,
    pub extra_args: Vec<String>,
}

/// UI 数字下拉框下标转可选值, 0 (未设置) 返回 None
pub fn numeric_preset(presets: &[u16], index: usize) -> Option<u16> {
    presets.get(index).copied().filter(|&v| v != 0)
}

/// UI 文本下拉框下标转可选值, 0 (默认) 返回 None
pub fn choice_preset(choices: &[&str], index: usize) -> Option<String> {
    if index == 0 {
        None
    } else {
        choices.get(index - 1).map(|s| s.to_string())
    }
}

/// 解析自定义参数输入框内容 (支持引号包裹含空格的参数)
pub fn parse_extra_args(raw: &str) -> Vec<String> {
    shlex::split(raw).unwrap_or_default()
}

/// 拼装传给 scrcpy 的完整命令行参数
pub fn build_scrcpy_args(serial: &str, options: &ScrcpyLaunchOptions) -> Vec<String> {
    let mut args = vec!["-s".to_string(), serial.to_string()];
    if let Some(max_size) = options.max_size {
        args.push(format!("--max-size={}", max_size));
    }
    if let Some(max_fps) = options.max_fps {
        args.push(format!("--max-fps={}", max_fps));
    }
    if let Some(bit_rate) = &options.bit_rate {
        args.push(format!("--video-bit-rate={}", bit_rate));
    }
    if let Some(codec) = &options.video_codec {
        args.push(format!("--video-codec={}", codec));
    }
    if !options.audio {
        args.push("--no-audio".to_string());
    }
    if options.stay_awake {
        args.push("--stay-awake".to_string());
    }
    args.extend(options.extra_args.iter().cloned());
    args
}

/// 启动官方 scrcpy 客户端子进程; 若同目录携带 adb 则通过 ADB 环境变量优先使用
pub async fn launch_scrcpy(
    serial: &str,
    options: &ScrcpyLaunchOptions,
) -> Result<tokio::process::Child, String> {
    let mut bin = PathBuf::from(&options.binary_path);
    if bin.is_dir() {
        bin = bin.join("scrcpy");
    }
    if !bin.is_file() {
        return Err(format!(
            "scrcpy 本体不存在或不可执行: {}",
            options.binary_path
        ));
    }

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(build_scrcpy_args(serial, options));
    if let Some(dir) = bin.parent().map(Path::to_path_buf) {
        let bundled_adb = dir.join("adb");
        if bundled_adb.is_file() {
            cmd.env("ADB", &bundled_adb);
        }
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    cmd.spawn().map_err(|e| format!("启动 scrcpy 失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_options() -> ScrcpyLaunchOptions {
        ScrcpyLaunchOptions {
            binary_path: "/Users/demo/scrcpy/scrcpy".to_string(),
            max_size: Some(1280),
            max_fps: Some(60),
            bit_rate: Some("8M".to_string()),
            video_codec: Some("h265".to_string()),
            audio: false,
            stay_awake: true,
            extra_args: vec!["--turn-screen-off".to_string()],
        }
    }

    #[test]
    fn build_args_contains_serial_and_selected_options() {
        let args = build_scrcpy_args("emulator-5554", &sample_options());
        let expected = [
            "-s",
            "emulator-5554",
            "--max-size=1280",
            "--max-fps=60",
            "--video-bit-rate=8M",
            "--video-codec=h265",
            "--no-audio",
            "--stay-awake",
            "--turn-screen-off",
        ];
        assert_eq!(args, expected);
    }

    #[test]
    fn unset_options_and_extra_audio_are_omitted() {
        let mut options = sample_options();
        options.max_size = None;
        options.max_fps = None;
        options.bit_rate = None;
        options.video_codec = None;
        options.stay_awake = false;
        options.audio = true;
        options.extra_args = Vec::new();

        let args = build_scrcpy_args("SERIAL", &options);
        // audio=true 走官方默认，不追加任何音频参数
        assert_eq!(args, vec!["-s", "SERIAL"]);
    }

    #[test]
    fn presets_map_ui_indexes_correctly() {
        assert_eq!(numeric_preset(&MAX_SIZE_PRESETS, 0), None);
        assert_eq!(numeric_preset(&MAX_SIZE_PRESETS, 4), Some(1280));
        assert_eq!(choice_preset(&BIT_RATE_PRESETS, 0), None);
        assert_eq!(choice_preset(&BIT_RATE_PRESETS, 2), Some("8M".to_string()));
    }

    #[test]
    fn extra_args_parse_respects_quotes() {
        let args = parse_extra_args("--video-codec-options=profile=1 --record=\"my file.mp4\"");
        assert_eq!(args.len(), 2);
        assert_eq!(args[1], "--record=my file.mp4");
    }
}
