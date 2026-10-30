use ikaros_adb::AdbClient;
use ikaros_domain::{DeviceRepository, LogLevel, LogcatRepository};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

#[tokio::test]
async fn test_logcat_parsing_and_live_stream() {
    // 1. 测试单行解析
    let sample = "10-27 19:22:30.123  1234  5678 D TestTag : Hello world test log";
    let entry = AdbClient::parse_logcat_line(sample).expect("解析日志应当成功");
    assert_eq!(entry.timestamp, "10-27 19:22:30.123");
    assert_eq!(entry.pid, 1234);
    assert_eq!(entry.tid, 5678);
    assert_eq!(entry.level, LogLevel::Debug);
    assert_eq!(entry.tag, "TestTag");
    assert_eq!(entry.message, "Hello world test log");

    // 2. 真实设备流拉取测试
    let adb = AdbClient::default();
    let devices = adb.list_devices().await.expect("获取设备失败");
    if devices.is_empty() {
        println!("未检测到设备，跳过测试");
        return;
    }

    let serial = &devices[0].serial;
    println!("在设备 [{}] 上测试 Logcat 实时日志流...", serial);

    let (tx, mut rx) = mpsc::channel(256);
    let (stop_tx, stop_rx) = watch::channel(false);

    let adb_clone = adb.clone();
    let serial_clone = serial.clone();
    tokio::spawn(async move {
        let _ = adb_clone
            .start_logcat_stream(&serial_clone, tx, stop_rx)
            .await;
    });

    let mut count = 0;
    while let Ok(Some(log)) = timeout(Duration::from_secs(5), rx.recv()).await {
        count += 1;
        println!(
            "[{}] [{}] ({}/{}) {}: {}",
            log.timestamp,
            log.level.as_str(),
            log.pid,
            log.tid,
            log.tag,
            log.message
        );
        if count >= 5 {
            break;
        }
    }

    let _ = stop_tx.send(true);
    assert!(count >= 1, "应当成功从设备捕获到实时日志");
    println!("Logcat 流式日志捕获测试全部通过！");
}
