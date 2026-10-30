use serde::{Deserialize, Serialize};

/// Android 设备当前的连接状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceStatus {
    /// 正常连接且已授权
    Device,
    /// 离线
    Offline,
    /// 未在设备上确认授权调试
    Unauthorized,
    /// 处于 Bootloader 模式
    Bootloader,
    /// 处于 Recovery 模式
    Recovery,
    /// 其它未知状态
    Unknown(String),
}

impl From<&str> for DeviceStatus {
    fn from(s: &str) -> Self {
        match s {
            "device" => Self::Device,
            "offline" => Self::Offline,
            "unauthorized" => Self::Unauthorized,
            "bootloader" => Self::Bootloader,
            "recovery" => Self::Recovery,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// 基础设备实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// 设备唯一序列号 (Serial)
    pub serial: String,
    /// 设备连接状态
    pub status: DeviceStatus,
    /// 产品型号 (如 Pixel_8_Pro)
    pub product: Option<String>,
    /// 型号名称 (如 Pixel 8 Pro)
    pub model: Option<String>,
    /// 设备名 (如 husky)
    pub device: Option<String>,
    /// 传输 ID (transport id)
    pub transport_id: Option<String>,
}

/// 设备的详细系统硬件与软件信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceDetail {
    /// 序列号
    pub serial: String,
    /// 品牌 (如 Google, Xiaomi)
    pub brand: String,
    /// 营销型号 (如 Pixel 8 Pro)
    pub model: String,
    /// Android 版本号 (如 "14" 或 "15")
    pub android_version: String,
    /// Android SDK API 级别 (如 34)
    pub sdk_api_level: u32,
    /// CPU 架构 (如 arm64-v8a)
    pub cpu_abi: String,
    /// 屏幕分辨率 (如 "1080x2400")
    pub screen_resolution: String,
    /// 屏幕密度 DPI
    pub screen_density: u32,
    /// 当前电池电量百分比 (0-100)
    pub battery_level: Option<u8>,
    /// 设备国际移动设备识别码 (IMEI)
    pub imei: Option<String>,
    /// 当前 Wi-Fi 连接的 IP 地址
    #[serde(default)]
    pub wifi_ip: Option<String>,
    /// 当前 Wi-Fi 连接使用的本机 MAC 地址，可能是随机地址
    #[serde(default)]
    pub wifi_mac: Option<String>,
    /// 本机蓝牙适配器的 MAC 地址
    #[serde(default)]
    pub bluetooth_mac: Option<String>,
}
