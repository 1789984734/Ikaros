use serde::{Deserialize, Serialize};

/// Android 应用过滤类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppFilterType {
    /// 第三方用户安装的应用
    ThirdParty,
    /// 系统预装应用
    System,
    /// 全部应用
    All,
}

/// Android 应用包详细信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    /// 应用在设备当前语言下的显示名称
    #[serde(default)]
    pub app_name: String,
    /// 应用唯一包名 (Package Name, 如 "com.tencent.mm")
    pub package_name: String,
    /// 本机缓存的应用图标；设备未提供图标时为空
    #[serde(default)]
    pub icon_path: Option<std::path::PathBuf>,
    /// 版本名称 (Version Name, 如 "8.0.40")
    pub version_name: String,
    /// 版本号 (Version Code)
    pub version_code: String,
    /// APK 在设备上的绝对物理路径
    pub apk_path: String,
    /// 是否为系统应用
    pub is_system: bool,
    /// 是否处于启用状态
    pub is_enabled: bool,
}
