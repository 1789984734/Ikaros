use ikaros_adb::AdbClient;
use ikaros_domain::DeviceRepository;

#[tokio::test]
async fn test_list_devices() {
    let client = AdbClient::default();
    match client.list_devices().await {
        Ok(devices) => {
            println!("成功发现 {} 台设备:", devices.len());
            for d in &devices {
                println!(
                    "- 序列号: {}, 状态: {:?}, 型号: {:?}",
                    d.serial, d.status, d.model
                );
            }
            assert!(!devices.is_empty(), "应当能检测到连接的设备");

            // 测试读取第一台设备的详细信息
            let first_serial = &devices[0].serial;
            let detail = client
                .get_device_detail(first_serial)
                .await
                .expect("获取设备详情应当成功");
            println!("设备详情: {:?}", detail);
            assert_eq!(detail.serial, *first_serial);
        }
        Err(e) => {
            panic!("连接 ADB 失败: {}", e);
        }
    }
}
