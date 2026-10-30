use ikaros_adb::AdbClient;
use ikaros_domain::{DeviceRepository, SyncRepository};

#[tokio::test]
async fn test_live_sync_file_operations() {
    let adb = AdbClient::default();
    let devices = adb.list_devices().await.expect("获取设备失败");
    if devices.is_empty() {
        println!("未检测到设备，跳过测试");
        return;
    }

    let serial = &devices[0].serial;
    println!("在设备 [{}] 上测试 ADB Sync 文件协议...", serial);

    // 1. 测试 LIST 协议：列出 /sdcard/ 目录
    let files = adb.list_directory(serial, "/sdcard").await.expect("列出 /sdcard 失败");
    println!("成功列出 /sdcard 目录下 {} 个文件/目录:", files.len());
    for f in files.iter().take(5) {
        println!("- [{}] {} (大小: {} 字节, 权限: {})", if f.is_dir { "DIR" } else { "FILE" }, f.name, f.size, f.permissions);
    }
    assert!(!files.is_empty());

    // 2. 测试 PUSH 协议：推送文件到 /data/local/tmp/test_sync.txt
    let local_test_file = std::env::temp_dir().join("test_ikaros_push.txt");
    let test_content = "Hello from Ikaros pure Rust ADB Sync protocol! 🚀";
    tokio::fs::write(&local_test_file, test_content).await.expect("写入本地测试文件失败");

    let remote_target = "/data/local/tmp/test_sync.txt";
    adb.push_file(serial, &local_test_file, remote_target, None)
        .await
        .expect("推送文件失败");
    println!("成功将文件推送到设备: {}", remote_target);

    // 3. 测试 RECV 协议：从设备拉取回本地
    let local_pulled_file = std::env::temp_dir().join("test_ikaros_pulled.txt");
    adb.pull_file(serial, remote_target, &local_pulled_file, None)
        .await
        .expect("拉取文件失败");
    println!("成功从设备拉取文件到: {:?}", local_pulled_file);

    let pulled_content = tokio::fs::read_to_string(&local_pulled_file).await.expect("读取拉取的文件失败");
    assert_eq!(pulled_content, test_content, "拉取的文件内容应当与推送的内容完全一致");
    println!("文件内容校验完全一致！ADB Sync LIST/PUSH/RECV 全部通过！");
}
