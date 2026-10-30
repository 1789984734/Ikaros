pub const DOCUMENTATION_VERSION: &str = "v5.0.1";
pub const UNKNOWN_DESCRIPTION: &str = "暂未收录此参数的说明，请查阅所用 scrcpy 版本的官方文档。";

pub struct ParameterHelp {
    pub name: &'static str,
    pub short: Option<char>,
    pub takes_value: bool,
    pub example: &'static str,
    pub description: &'static str,
    pub source: &'static str,
}

impl ParameterHelp {
    pub fn documentation_url(&self) -> String {
        format!(
            "https://github.com/Genymobile/scrcpy/blob/{DOCUMENTATION_VERSION}/doc/{}",
            self.source
        )
    }
}

pub const PARAMETERS: &[ParameterHelp] = &[
    ParameterHelp {
        name: "--turn-screen-off",
        short: Some('S'),
        takes_value: false,
        example: "--turn-screen-off",
        description: "启动投屏时关闭设备屏幕，电脑端继续显示和操控。可与 -w 组合为 -Sw。",
        source: "device.md#turn-screen-off",
    },
    ParameterHelp {
        name: "--stay-awake",
        short: Some('w'),
        takes_value: false,
        example: "--stay-awake",
        description: "设备充电时保持唤醒，退出后恢复原设置；设备未充电时无效。",
        source: "device.md#stay-awake",
    },
    ParameterHelp {
        name: "--power-off-on-close",
        short: None,
        takes_value: false,
        example: "--power-off-on-close",
        description: "退出 scrcpy 时关闭设备屏幕。",
        source: "device.md#power-off-on-close",
    },
    ParameterHelp {
        name: "--keep-active",
        short: None,
        takes_value: false,
        example: "--keep-active",
        description: "周期性模拟用户活动，防止设备因长时间无操作而自动息屏。",
        source: "device.md#keep-active",
    },
    ParameterHelp {
        name: "--no-audio",
        short: None,
        takes_value: false,
        example: "--no-audio",
        description: "关闭音频转发，录屏文件中也不会包含设备音频。",
        source: "audio.md#no-audio",
    },
    ParameterHelp {
        name: "--show-touches",
        short: Some('t'),
        takes_value: false,
        example: "--show-touches",
        description: "显示手指在设备屏幕上的触摸位置；仅显示实体触摸，退出后恢复原设置。",
        source: "device.md#show-touches",
    },
    ParameterHelp {
        name: "--no-power-on",
        short: None,
        takes_value: false,
        example: "--no-power-on",
        description: "启动 scrcpy 时不自动唤醒设备屏幕。",
        source: "device.md#power-on-on-start",
    },
    ParameterHelp {
        name: "--screen-off-timeout",
        short: None,
        takes_value: true,
        example: "--screen-off-timeout=300",
        description: "临时设置自动息屏时间，单位为秒；退出后恢复原设置。",
        source: "device.md#screen-off-timeout",
    },
    ParameterHelp {
        name: "--max-size",
        short: Some('m'),
        takes_value: true,
        example: "--max-size=1280",
        description: "限制画面宽、高的最大值，普通投屏保持宽高比；降低分辨率可减轻编码和传输负担。",
        source: "video.md#size",
    },
    ParameterHelp {
        name: "--max-fps",
        short: None,
        takes_value: true,
        example: "--max-fps=60",
        description: "限制最高捕获帧率，单位为 FPS；实际帧率随屏幕内容变化。",
        source: "video.md#frame-rate",
    },
    ParameterHelp {
        name: "--video-bit-rate",
        short: Some('b'),
        takes_value: true,
        example: "--video-bit-rate=8M",
        description: "设置视频码率，支持 K、M 后缀；提高码率通常可改善画质，也会增加带宽占用。",
        source: "video.md#bit-rate",
    },
    ParameterHelp {
        name: "--video-codec",
        short: None,
        takes_value: true,
        example: "--video-codec=h265",
        description: "选择视频编码，例如 h264、h265 或 av1；可用编码取决于设备支持情况。",
        source: "video.md#codec",
    },
    ParameterHelp {
        name: "--always-on-top",
        short: None,
        takes_value: false,
        example: "--always-on-top",
        description: "让投屏窗口始终置顶。",
        source: "window.md#always-on-top",
    },
    ParameterHelp {
        name: "--fullscreen",
        short: Some('f'),
        takes_value: false,
        example: "--fullscreen",
        description: "启动时直接进入全屏模式，之后仍可使用快捷键切换。",
        source: "window.md#fullscreen",
    },
    ParameterHelp {
        name: "--window-borderless",
        short: None,
        takes_value: false,
        example: "--window-borderless",
        description: "隐藏投屏窗口的边框和标题栏。",
        source: "window.md#borderless",
    },
    ParameterHelp {
        name: "--record",
        short: Some('r'),
        takes_value: true,
        example: "--record=demo.mp4",
        description: "将投屏录制到电脑上的文件，默认按扩展名选择容器格式；音频关闭时只录制视频。",
        source: "recording.md",
    },
    ParameterHelp {
        name: "--new-display",
        short: None,
        takes_value: true,
        example: "--new-display=1920x1080",
        description:
            "创建并投屏一个独立虚拟显示屏，退出时销毁；没有桌面启动器时需配合 --start-app。",
        source: "virtual-display.md#new-display",
    },
    ParameterHelp {
        name: "--start-app",
        short: None,
        takes_value: true,
        example: "--start-app=com.android.settings",
        description:
            "启动指定包名的 Android 应用；可与 --new-display 配合，在虚拟显示屏中运行应用。",
        source: "device.md#start-android-app",
    },
    ParameterHelp {
        name: "--no-control",
        short: Some('n'),
        takes_value: false,
        example: "--no-control",
        description: "仅观看投屏，禁用键盘、鼠标控制及文件拖放等设备交互。",
        source: "control.md#read-only",
    },
    ParameterHelp {
        name: "--keyboard",
        short: None,
        takes_value: true,
        example: "--keyboard=uhid",
        description:
            "选择键盘输入模式；uhid 模拟实体键盘，使用前需在设备上配置对应的物理键盘布局。",
        source: "keyboard.md",
    },
    ParameterHelp {
        name: "--no-clipboard-autosync",
        short: None,
        takes_value: false,
        example: "--no-clipboard-autosync",
        description: "关闭设备与电脑之间的自动剪贴板同步。",
        source: "control.md#copy-paste",
    },
];

pub fn describe_argument(argument: &str) -> String {
    let argument = argument.trim();
    if argument.starts_with("--") {
        let name = argument.split('=').next().unwrap_or(argument);
        return PARAMETERS
            .iter()
            .find(|parameter| parameter.name == name)
            .map(|parameter| parameter.description.to_string())
            .unwrap_or_else(|| UNKNOWN_DESCRIPTION.to_string());
    }
    describe_short_options(argument).unwrap_or_else(|| UNKNOWN_DESCRIPTION.to_string())
}

fn describe_short_options(argument: &str) -> Option<String> {
    let mut descriptions = Vec::new();
    for short in argument.strip_prefix('-')?.chars() {
        let parameter = PARAMETERS
            .iter()
            .find(|parameter| parameter.short == Some(short))?;
        descriptions.push(format!("{}：{}", parameter.name, parameter.description));
        // The remaining characters belong to the value of options like -m1280.
        if parameter.takes_value {
            break;
        }
    }
    (!descriptions.is_empty()).then(|| descriptions.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_short_flags_describe_both_actions() {
        let description = describe_argument("-Sw");
        assert!(description.contains("--turn-screen-off"));
        assert!(description.contains("--stay-awake"));
        assert!(description.contains("设备未充电时无效"));
    }

    #[test]
    fn values_and_short_option_values_do_not_affect_lookup() {
        assert_eq!(
            describe_argument("--record=a b.mp4"),
            describe_argument("--record=demo.mkv")
        );
        let description = describe_argument("-wm1280");
        assert!(description.contains("--stay-awake"));
        assert!(description.contains("--max-size"));
        assert_ne!(describe_argument("-t"), UNKNOWN_DESCRIPTION);
    }

    #[test]
    fn unknown_arguments_are_not_assigned_a_similar_options_help() {
        for argument in ["", "-", "--", "demo.mp4", "--recording=demo.mp4", "-Sq"] {
            assert_eq!(describe_argument(argument), UNKNOWN_DESCRIPTION);
        }
    }
}
