use super::*;
use ikaros_domain::PackageRepository;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

async fn read_request(stream: &mut TcpStream) -> String {
    let mut length = [0; 4];
    stream.read_exact(&mut length).await.unwrap();
    let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await.unwrap();
    String::from_utf8(bytes).unwrap()
}

async fn fake_adb(outputs: &[&str]) -> (AdbClient, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = AdbClient::new(listener.local_addr().unwrap());
    let outputs = outputs
        .iter()
        .map(|output| output.to_string())
        .collect::<Vec<_>>();
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
            let command = read_request(&mut stream).await;
            commands.push(command.strip_prefix("shell:").unwrap().to_owned());
            stream.write_all(b"OKAY").await.unwrap();
            stream.write_all(output.as_bytes()).await.unwrap();
        }
        commands
    });
    (client, server)
}

#[tokio::test]
async fn settings_uses_the_system_home_intent_and_returns_to_the_top() {
    let (adb, server) = fake_adb(&[
        "priority=1 preferredOrder=0\ncom.android.settings/.homepage.SettingsHomepageActivity\n",
        "Starting: Intent {...}\nIKAROS_LAUNCH_EXIT:0\n",
    ])
    .await;
    adb.launch_package("test-device", SETTINGS_PACKAGE)
        .await
        .unwrap();
    let commands = server.await.unwrap();
    assert_eq!(commands[0], "pm resolve-activity --brief --user current -a android.settings.SETTINGS -p com.android.settings");
    assert_eq!(commands[1], "am start --user current -a android.settings.SETTINGS -f 0x14000000 -n 'com.android.settings/.homepage.SettingsHomepageActivity'; ikaros_launch_status=$?; echo IKAROS_LAUNCH_EXIT:$ikaros_launch_status");
}

#[tokio::test]
async fn settings_with_an_oem_launcher_subpage_falls_back_to_its_home_component() {
    let (adb, server) = fake_adb(&[
        "No activity found\n",
        "priority=0 isDefault=false\ncom.android.settings/.Settings\n",
        "Starting: Intent {...}\nIKAROS_LAUNCH_EXIT:0\n",
    ])
    .await;
    adb.launch_package("test-device", SETTINGS_PACKAGE)
        .await
        .unwrap();
    let commands = server.await.unwrap();
    assert_eq!(
        commands[1],
        "pm resolve-activity --brief --user current -n com.android.settings/.Settings"
    );
    assert!(commands[2].contains("-n 'com.android.settings/.Settings';"));
    assert!(commands
        .iter()
        .all(|command| !command.contains("LAUNCHER") && !command.contains("monkey")));
}

#[tokio::test]
async fn ordinary_apps_use_the_resolved_launcher_and_quote_inner_class_names() {
    let (adb, server) = fake_adb(&[
        "priority=0\ncom.example.app/.Main$Launcher\n",
        "Starting: Intent {...}\nWarning: Activity not started, its current task has been brought to the front\nIKAROS_LAUNCH_EXIT:0\n",
    ]).await;
    adb.launch_package("test-device", "com.example.app")
        .await
        .unwrap();
    let commands = server.await.unwrap();
    assert!(commands[0].contains("android.intent.category.LAUNCHER -p com.example.app"));
    assert_eq!(commands[1], "am start --user current -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -f 0x10200000 -n 'com.example.app/.Main$Launcher'; ikaros_launch_status=$?; echo IKAROS_LAUNCH_EXIT:$ikaros_launch_status");
}

#[tokio::test]
async fn missing_home_or_launcher_reports_a_failure_without_starting_another_page() {
    let (adb, server) = fake_adb(&["No activity found\n", "No activity found\n"]).await;
    let error = adb
        .launch_package("test-device", SETTINGS_PACKAGE)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("设置首页"));
    assert_eq!(server.await.unwrap().len(), 2);
    let (adb, server) = fake_adb(&["No activity found\n"]).await;
    assert!(adb
        .launch_package("test-device", "com.example.service")
        .await
        .is_err());
    assert_eq!(server.await.unwrap().len(), 1);
}

#[test]
fn resolver_output_cannot_select_another_package_or_inject_shell_commands() {
    for package in [
        "",
        "com.example;reboot",
        "com.example.$(id)",
        "com..example",
    ] {
        assert!(!valid_package_name(package));
    }
    for output in [
        "android/.ResolverActivity",
        "com.example/.Main'; reboot",
        "com.example/.Main\nNo activity found",
    ] {
        assert!(parse_component(output, "com.other").is_none());
    }
    assert!(parse_component("com.example/.Main'; reboot", "com.example").is_none());
}

#[test]
fn launch_denials_and_timeouts_are_not_reported_as_success() {
    for output in [
        "",
        "Status: timeout",
        "Error type 3\nError: Activity class does not exist.",
        "Starting: Intent {...}\njava.lang.SecurityException: Permission Denial\nIKAROS_LAUNCH_EXIT:0",
        "Starting: Intent {...}\nIKAROS_LAUNCH_EXIT:1",
        "Starting: Intent {...}\n",
    ] {
        assert!(check_start_result(output).is_err(), "{output}");
    }
}
