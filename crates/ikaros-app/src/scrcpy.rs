//! 外部官方 scrcpy 客户端的启动参数拼装与进程管理

use serde::{Deserialize, Serialize};
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

// Required operands in scrcpy v5.0.1. A separate operand may itself start with '-'.
const LONG_OPTIONS_WITH_VALUE: &str = "angle audio-bit-rate audio-buffer audio-codec \
    audio-codec-options audio-encoder audio-source audio-output-buffer video-bit-rate \
    background-color camera-ar camera-facing camera-fps camera-id camera-size camera-zoom \
    capture-orientation crop display-id display-ime-policy display-orientation gamepad hwdec \
    keyboard max-size max-fps min-size-alignment mouse mouse-bind orientation port push-target \
    record record-format record-orientation render-driver render-fit serial screen-off-timeout \
    shortcut-mod start-app time-limit tunnel-host tunnel-port verbosity v4l2-sink v4l2-buffer \
    video-buffer video-codec video-codec-options video-encoder video-source window-title \
    window-x window-y window-width window-height";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScrcpyPreferences {
    pub max_size_index: i32,
    pub max_fps_index: i32,
    pub bit_rate_index: i32,
    pub codec_index: i32,
    pub audio: bool,
    pub stay_awake: bool,
    pub extra_args: String,
}

impl Default for ScrcpyPreferences {
    fn default() -> Self {
        Self {
            max_size_index: 0,
            max_fps_index: 2,
            bit_rate_index: 2,
            codec_index: 0,
            audio: false,
            stay_awake: true,
            extra_args: String::new(),
        }
    }
}

impl ScrcpyPreferences {
    pub fn launch_options(&self, binary_path: String) -> ScrcpyLaunchOptions {
        ScrcpyLaunchOptions {
            binary_path,
            max_size: numeric_preset(&MAX_SIZE_PRESETS, self.max_size_index as usize),
            max_fps: numeric_preset(&MAX_FPS_PRESETS, self.max_fps_index as usize),
            bit_rate: choice_preset(&BIT_RATE_PRESETS, self.bit_rate_index as usize),
            video_codec: choice_preset(&VIDEO_CODECS, self.codec_index as usize),
            audio: self.audio,
            stay_awake: self.stay_awake,
            extra_args: parse_extra_args(&self.extra_args),
        }
    }
}

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

impl ScrcpyLaunchOptions {
    pub fn uses_h265_video(&self) -> bool {
        let mut codec = self.video_codec.as_deref();
        for option in extra_options(&self.extra_args) {
            match option.name {
                "--no-video" | "--otg" => return false,
                "--video-codec" => codec = option.value,
                _ => {}
            }
        }
        codec == Some("h265")
    }

    pub fn use_h264(&mut self) {
        self.video_codec = Some("h264".into());
        self.extra_args = extra_options(&self.extra_args)
            .filter(|option| {
                !matches!(
                    option.name,
                    "--video-codec" | "--video-encoder" | "--video-codec-options"
                )
            })
            .flat_map(|option| option.raw.iter().cloned())
            .collect();
    }

    pub fn recording_path(&self) -> Option<&str> {
        extra_options(&self.extra_args)
            .filter(|option| option.name == "--record")
            .last()
            .and_then(|option| option.value)
    }

    pub fn kills_adb_on_exit(&self) -> bool {
        extra_options(&self.extra_args).any(|option| option.name == "--kill-adb-on-close")
    }

    pub fn continue_recording(&mut self, previous_pid: u32) {
        let Some(path) = self.recording_path().map(Path::new) else {
            return;
        };
        let mut name = path.file_stem().unwrap_or_default().to_os_string();
        let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        name.push(format!("-recovery-{previous_pid}-{stamp}"));
        if let Some(extension) = path.extension() {
            name.push(".");
            name.push(extension);
        }
        let path = path.with_file_name(name);
        let end_of_options = extra_options(&self.extra_args)
            .find(|option| option.name == "--")
            .map_or(self.extra_args.len(), |option| {
                self.extra_args.len() - option.raw.len()
            });
        self.extra_args
            .insert(end_of_options, format!("--record={}", path.display()));
    }
}

struct ExtraOption<'a> {
    name: &'a str,
    value: Option<&'a str>,
    raw: &'a [String],
}

fn extra_options(mut args: &[String]) -> impl Iterator<Item = ExtraOption<'_>> {
    std::iter::from_fn(move || {
        let (argument, rest) = args.split_first()?;
        let (name, mut value, takes_value) = argument_parts(argument);
        let length = if name == "--" {
            args.len()
        } else if takes_value && value.is_none() && !rest.is_empty() {
            value = rest.first().map(String::as_str);
            2
        } else {
            1
        };
        let (raw, remaining) = args.split_at(length);
        args = remaining;
        Some(ExtraOption { name, value, raw })
    })
}

fn argument_parts(argument: &str) -> (&str, Option<&str>, bool) {
    if argument.starts_with("--") {
        if let Some((name, value)) = argument.split_once('=') {
            return (name, Some(value), false);
        }
        let takes_value = LONG_OPTIONS_WITH_VALUE
            .split_ascii_whitespace()
            .any(|option| option == &argument[2..]);
        return (argument, None, takes_value);
    }
    if let Some((flag, value)) = short_option_with_value(argument) {
        let name = if flag == 'r' { "--record" } else { argument };
        return (name, (!value.is_empty()).then_some(value), true);
    }
    (argument, None, false)
}

fn short_option_with_value(argument: &str) -> Option<(char, &str)> {
    let flags = argument.strip_prefix('-')?;
    if flags.starts_with('-') {
        return None;
    }
    // scrcpy's short options with required values, including bundles such as -SrFILE.
    flags
        .char_indices()
        .find(|(_, flag)| "bmprsV".contains(*flag))
        .map(|(index, flag)| (flag, &flags[index + flag.len_utf8()..]))
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
        if bin.join("scrcpy.exe").is_file() {
            bin = bin.join("scrcpy.exe");
        } else if bin.join("scrcpy").is_file() {
            bin = bin.join("scrcpy");
        } else if cfg!(target_os = "windows") {
            bin = bin.join("scrcpy.exe");
        } else {
            bin = bin.join("scrcpy");
        }
    } else if !bin.is_file() {
        if let Some(with_exe) = (!bin.to_string_lossy().ends_with(".exe"))
            .then(|| bin.with_extension("exe"))
            .filter(|p| p.is_file())
        {
            bin = with_exe;
        }
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
        let bundled_adb = if dir.join("adb.exe").is_file() {
            Some(dir.join("adb.exe"))
        } else if dir.join("adb").is_file() {
            Some(dir.join("adb"))
        } else {
            None
        };
        if let Some(adb) = bundled_adb {
            cmd.env("ADB", &adb);
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

    #[test]
    fn h264_recovery_preserves_other_launch_options_and_discards_hevc_overrides() {
        let mut preferred = sample_options();
        preferred.video_codec = None;
        preferred.extra_args = parse_extra_args(
            "--video-codec=h264 --video-codec h265 --video-encoder=OMX.sprd.h265.encoder \
             --video-codec-options profile=2 --window-title='Test device' --max-fps=30",
        );
        assert!(preferred.uses_h265_video());
        let mut fallback = preferred.clone();
        fallback.use_h264();
        assert!(!fallback.uses_h265_video());
        assert_eq!(fallback.video_codec.as_deref(), Some("h264"));
        assert_eq!(
            fallback.extra_args,
            ["--window-title=Test device", "--max-fps=30"]
        );
        assert_eq!(fallback.bit_rate, preferred.bit_rate);
        assert!(preferred.uses_h265_video());

        preferred.extra_args.push("--no-video".into());
        assert!(!preferred.uses_h265_video());
    }

    #[test]
    fn recovered_recordings_use_a_separate_file_with_the_original_format() {
        for given_args in [
            "--record=clip.mp4",
            "--record clip.mp4",
            "-Srclip.mp4",
            "-r clip.mp4",
        ] {
            let mut options = sample_options();
            options.extra_args = parse_extra_args(given_args);
            assert_eq!(options.recording_path(), Some("clip.mp4"));
            options.continue_recording(345);
            let continuation = options.recording_path().unwrap();
            assert!(continuation.starts_with("clip-recovery-345-"));
            assert!(continuation.ends_with(".mp4"));
        }
    }

    #[test]
    fn recovery_leaves_flag_like_operands_untouched() {
        let mut options = sample_options();
        options.extra_args = parse_extra_args(
            "--window-title --record=title.mp4 --push-target --video-encoder=path --video-codec=h265",
        );
        assert!(options.recording_path().is_none());
        assert!(options.uses_h265_video());
        options.use_h264();
        assert_eq!(
            options.extra_args,
            [
                "--window-title",
                "--record=title.mp4",
                "--push-target",
                "--video-encoder=path"
            ]
        );
    }
}
