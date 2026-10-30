use ikaros_adb::AdbClient;
use ikaros_domain::{AppFilterType, DeviceRepository, PackageRepository};

#[tokio::test]
async fn test_live_app_names_and_icons() {
    let adb = AdbClient::default();
    let devices = adb.list_devices().await.expect("获取设备失败");
    for device in devices {
        let apps = adb.list_packages(&device.serial, AppFilterType::All)
            .await.expect("查询应用信息失败");
        let user_id = adb.execute_shell(&device.serial, "am get-current-user")
            .await.expect("查询当前用户失败").trim().parse::<u32>()
            .expect("当前用户 ID 应为非负整数");
        let output = adb.execute_shell(&device.serial, &format!("pm list packages --user {user_id}"))
            .await.expect("查询包名失败");
        let expected: std::collections::HashSet<_> = output.lines()
            .filter_map(|line| line.trim().strip_prefix("package:")).collect();
        let actual: std::collections::HashSet<_> = apps.iter()
            .map(|app| app.package_name.as_str()).collect();
        assert_eq!(actual, expected, "应用列表不能漏包");
        assert!(apps.iter().all(|app| !app.app_name.is_empty()));
        assert!(apps.iter().any(|app| app.app_name != app.package_name));
        let mut icons = 0;
        for app in &apps {
            if let Some(path) = &app.icon_path {
                let png = std::fs::read(path).expect("读取图标失败");
                assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
                assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 96);
                assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 96);
                icons += 1;
            }
        }
        assert!(icons > 0, "应能读取真实应用图标");
        let sample = apps.iter().find(|app| !app.is_system).unwrap_or(&apps[0]);
        let (name, code) = adb.get_package_detail(&device.serial, &sample.package_name)
            .await.expect("查询版本失败");
        assert_eq!(sample.version_name, if name == "null" { "" } else { &name });
        assert_eq!(sample.version_code, code);
        println!("{}: {} 个应用，{} 个真实图标", device.serial, apps.len(), icons);
    }
}

#[tokio::test]
async fn test_live_package_listing() {
    let adb = AdbClient::default();
    let devices = adb.list_devices().await.expect("获取设备失败");
    if devices.is_empty() {
        println!("未检测到设备，跳过测试");
        return;
    }

    let serial = &devices[0].serial;
    println!("在设备 [{}] 上测试 Package 应用管理协议...", serial);

    // 1. 查询第三方应用
    let third_party_apps = adb
        .list_packages(serial, AppFilterType::ThirdParty)
        .await
        .expect("查询第三方应用失败");
    println!("已安装第三方应用数量: {}", third_party_apps.len());
    for app in third_party_apps.iter().take(5) {
        println!("- 包名: {}, 路径: {}", app.package_name, app.apk_path);
    }

    // 2. 查询系统应用
    let system_apps = adb
        .list_packages(serial, AppFilterType::System)
        .await
        .expect("查询系统应用失败");
    println!("已安装系统应用数量: {}", system_apps.len());
    assert!(!system_apps.is_empty(), "系统应用列表不应为空");

    // 3. 测试查询某个具体包的详情版本
    if let Some(first) = third_party_apps.first().or_else(|| system_apps.first()) {
        let (ver_name, ver_code) = adb
            .get_package_detail(serial, &first.package_name)
            .await
            .expect("查询包详情失败");
        println!(
            "包 [{}] 版本信息: VersionName={}, VersionCode={}",
            first.package_name, ver_name, ver_code
        );
    }

    println!("PackageRepository 接口在真实设备上测试通过！");
}

#[tokio::test]
async fn test_live_apk_install_flow() {
    use ikaros_domain::SyncRepository;
    use std::path::PathBuf;

    let adb = AdbClient::default();
    let devices = adb.list_devices().await.expect("获取设备失败");
    if devices.is_empty() {
        println!("未检测到设备，跳过测试");
        return;
    }

    let serial = &devices[0].serial;
    let apps = adb
        .list_packages(serial, AppFilterType::ThirdParty)
        .await
        .expect("查询应用失败");

    if let Some(target_app) = apps.first() {
        println!("测试提取现有包 [{}] 模拟拖拽安装流程...", target_app.package_name);
        let temp_local_apk = PathBuf::from("/tmp/test_live_drag_package.apk");

        // 1. 先将设备上的 APK 拉取到本地
        adb.pull_file(serial, &target_app.apk_path, &temp_local_apk, None)
            .await
            .expect("拉取 APK 失败");
        assert!(temp_local_apk.exists(), "本地临时 APK 文件应存在");

        // 2. 调用 install_apk 重新覆盖安装（模拟用户拖拽 APK 释放安装）
        println!("开始执行 install_apk 模拟拖拽释放覆盖安装...");
        let install_result = adb.install_apk(serial, &temp_local_apk).await;
        assert!(install_result.is_ok(), "APK 安装应返回成功: {:?}", install_result);
        println!("APK 拖拽安装与覆盖校验成功！");

        // 3. 清理本地临时文件
        let _ = std::fs::remove_file(&temp_local_apk);
    } else {
        println!("设备上无第三方应用，跳过拖拽安装覆盖测试");
    }
}
