use super::*;
use ikaros_adb::AdbClient;
use ikaros_domain::{AppFilterType, AppInfo, Device, DeviceStatus, PackageRepository};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, UnboundedReceiver},
    task::JoinHandle,
};

fn device(serial: &str) -> Device {
    Device {
        serial: serial.into(),
        status: DeviceStatus::Device,
        model: None,
        product: None,
        device: None,
        transport_id: None,
    }
}

fn app(package_name: &str, is_system: bool) -> AppInfo {
    AppInfo {
        app_name: package_name.into(),
        package_name: package_name.into(),
        icon_path: None,
        version_name: "1.0".into(),
        version_code: "1".into(),
        apk_path: "/data/app/base.apk".into(),
        is_system,
        is_enabled: true,
    }
}

async fn store(adb: Arc<AdbClient>) -> (AppStore, UnboundedReceiver<AppIntent>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut store = AppStore::new(
        Arc::new(DeviceUseCases::new(adb.clone())),
        Arc::new(FileUseCases::new(adb.clone())),
        Arc::new(AppUseCases::new(adb.clone())),
        Arc::new(LogcatUseCases::new(adb.clone())),
        tx,
        directory.path().join("logs"),
        Arc::new(PerformanceUseCases::new(adb)),
    );
    store.state.devices = vec![device("test-device")];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap(),
        Some(AppIntent::LogRecordingUpdated { .. })
    ));
    (store, rx, directory)
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut length = [0; 4];
    stream.read_exact(&mut length).await.unwrap();
    let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await.unwrap();
    String::from_utf8(bytes).unwrap()
}

async fn fake_adb(outputs: Vec<&'static str>) -> (Arc<AdbClient>, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adb = Arc::new(AdbClient::new(listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let mut commands = Vec::new();
        for output in outputs {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                read_request(&mut stream).await,
                "host:transport:test-device"
            );
            stream.write_all(b"OKAY").await.unwrap();
            commands.push(read_request(&mut stream).await);
            stream.write_all(b"OKAY").await.unwrap();
            stream.write_all(output.as_bytes()).await.unwrap();
        }
        commands
    });
    (adb, server)
}

async fn next_intent(rx: &mut UnboundedReceiver<AppIntent>) -> AppIntent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let intent = rx.recv().await.unwrap();
            if !matches!(intent, AppIntent::LogRecordingUpdated { .. }) {
                return intent;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn uninstall_uses_the_app_type_and_refreshes_the_current_filter_after_success() {
    for is_system in [false, true] {
        let outputs = if is_system {
            vec!["10\n", "Success\r\n"]
        } else {
            vec!["Success\r\n"]
        };
        let (adb, server) = fake_adb(outputs).await;
        let (mut store, mut rx, _directory) = store(adb).await;
        store.state.app_list = vec![
            app("com.example.target", is_system),
            app("com.example.keep", false),
        ];
        store.state.app_search_query = "example".into();

        store
            .dispatch(AppIntent::UninstallApp("com.example.target".into()))
            .await;
        assert!(store.state.is_busy);
        assert!(store.state.status_message.contains("正在卸载"));
        store
            .dispatch(AppIntent::UninstallApp("com.example.target".into()))
            .await;
        store.state.app_filter = AppFilterType::System;
        let result = next_intent(&mut rx).await;
        assert!(
            matches!(&result, AppIntent::AppUninstalled { serial, result: Ok(()), .. } if serial == "test-device")
        );
        store.dispatch(result).await;

        let expected = if is_system {
            vec![
                "shell:am get-current-user",
                "shell:pm uninstall --user 10 com.example.target",
            ]
        } else {
            vec!["shell:pm uninstall com.example.target"]
        };
        assert_eq!(server.await.unwrap(), expected);
        assert!(!store.state.is_busy);
        assert_eq!(store.state.app_list.len(), 1);
        assert_eq!(store.state.app_list[0].package_name, "com.example.keep");
        assert_eq!(store.state.app_search_query, "example");
        assert!(store.state.status_message.contains("已成功卸载"));
        assert!(matches!(
            next_intent(&mut rx).await,
            AppIntent::LoadPackages(AppFilterType::System)
        ));
    }
}

#[tokio::test]
async fn rejected_or_empty_uninstalls_keep_the_app_and_report_the_failure() {
    for output in [
        "Failure [DELETE_FAILED_USER_RESTRICTED]\n",
        "Failure [Success]\n",
        "",
    ] {
        let (adb, server) = fake_adb(vec!["0\n", output]).await;
        let (mut store, mut rx, _directory) = store(adb).await;
        store.state.app_list = vec![app("com.example.target", true)];
        store
            .dispatch(AppIntent::UninstallApp("com.example.target".into()))
            .await;
        let result = next_intent(&mut rx).await;
        assert!(matches!(
            &result,
            AppIntent::AppUninstalled { result: Err(_), .. }
        ));
        store.dispatch(result).await;

        server.await.unwrap();
        assert!(!store.state.is_busy);
        assert_eq!(store.state.app_list.len(), 1);
        assert!(store.state.status_message.contains("卸载应用失败"));
        assert_eq!(store.state.app_status_message, store.state.status_message);
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn an_unknown_current_user_never_falls_back_to_uninstalling_for_all_users() {
    let (adb, server) = fake_adb(vec!["Error: unable to read user\n"]).await;
    let (mut store, mut rx, _directory) = store(adb).await;
    store.state.app_list = vec![app("com.example.target", true)];
    store
        .dispatch(AppIntent::UninstallApp("com.example.target".into()))
        .await;
    let result = next_intent(&mut rx).await;
    store.dispatch(result).await;

    assert_eq!(server.await.unwrap(), vec!["shell:am get-current-user"]);
    assert_eq!(store.state.app_list.len(), 1);
    assert!(!store.state.is_busy);
    assert!(store.state.status_message.contains("无法读取设备当前用户"));
}

#[tokio::test]
async fn uninstall_requires_a_connected_device_and_an_app_in_the_current_list() {
    let adb = Arc::new(AdbClient::new("127.0.0.1:1".parse().unwrap()));
    let (mut store, _rx, _directory) = store(adb).await;
    store.state.devices.clear();
    store
        .dispatch(AppIntent::UninstallApp("com.example.target".into()))
        .await;
    assert!(!store.state.is_busy);
    assert!(store.state.status_message.contains("请先连接"));

    store.state.devices = vec![device("test-device")];
    store
        .dispatch(AppIntent::UninstallApp("com.example.target".into()))
        .await;
    assert!(!store.state.is_busy);
    assert!(store.state.status_message.contains("刷新后重试"));
}

#[tokio::test]
async fn uninstall_completion_from_another_device_does_not_change_the_current_list() {
    let adb = Arc::new(AdbClient::new("127.0.0.1:1".parse().unwrap()));
    let (mut store, mut rx, _directory) = store(adb).await;
    store.state.app_list = vec![app("com.example.target", true)];
    store.state.is_busy = true;
    let status = store.state.status_message.clone();
    store
        .dispatch(AppIntent::AppUninstalled {
            serial: "other-device".into(),
            package_name: "com.example.target".into(),
            result: Ok(()),
        })
        .await;

    assert!(store.state.is_busy);
    assert_eq!(store.state.app_list.len(), 1);
    assert_eq!(store.state.status_message, status);
    while let Ok(intent) = rx.try_recv() {
        assert!(!matches!(intent, AppIntent::LoadPackages(_)));
    }
}

#[tokio::test]
async fn invalid_package_names_are_rejected_before_sending_an_uninstall_command() {
    let adb = AdbClient::new("127.0.0.1:1".parse().unwrap());
    for package in [
        "",
        "com.example;reboot",
        "com.example.$(id)",
        "com..example",
    ] {
        let error = adb
            .uninstall_package("test-device", package, true)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("无效的应用包名"));
    }
}

#[tokio::test]
async fn stale_package_lists_do_not_replace_the_current_device_or_filter() {
    let adb = Arc::new(AdbClient::new("127.0.0.1:1".parse().unwrap()));
    let (mut store, _rx, _directory) = store(adb).await;
    store.state.app_filter = AppFilterType::System;
    store.state.app_list = vec![app("com.example.current", true)];
    store.state.is_busy = true;
    for (serial, filter) in [
        ("other-device", AppFilterType::System),
        ("test-device", AppFilterType::ThirdParty),
    ] {
        for result in [
            Ok(vec![app("com.example.stale", false)]),
            Err("stale error".into()),
        ] {
            store
                .dispatch(AppIntent::PackagesLoaded {
                    serial: serial.into(),
                    filter,
                    result,
                })
                .await;
            assert!(store.state.is_busy);
            assert_eq!(store.state.app_list[0].package_name, "com.example.current");
        }
    }
    store
        .dispatch(AppIntent::PackagesLoaded {
            serial: "test-device".into(),
            filter: AppFilterType::System,
            result: Ok(vec![app("com.example.fresh", true)]),
        })
        .await;
    assert!(!store.state.is_busy);
    assert_eq!(store.state.app_list[0].package_name, "com.example.fresh");
}
