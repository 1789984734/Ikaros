use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

async fn read_request(stream: &mut TcpStream) -> String {
    let mut length = [0; 4];
    stream.read_exact(&mut length).await.unwrap();
    let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await.unwrap();
    String::from_utf8(bytes).unwrap()
}

#[tokio::test]
async fn fallback_uses_the_current_user_and_recognizes_updated_and_vendor_system_apps() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adb = AdbClient::new(listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let responses = [
            None,
            Some(""),
            Some("10\n"),
            Some("package:/data/app/base.apk=com.example.updated\npackage:/vendor/app/base.apk=com.example.vendor\npackage:/data/app/user.apk=com.example.user\n"),
            Some("package:com.example.updated\npackage:com.example.vendor\n"),
            Some(""),
        ];
        let mut commands = Vec::new();
        for response in responses {
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
            match response {
                Some(output) => {
                    stream.write_all(b"OKAY").await.unwrap();
                    stream.write_all(output.as_bytes()).await.unwrap();
                }
                None => stream.write_all(b"FAIL0006denied").await.unwrap(),
            }
        }
        commands
    });

    let apps = adb
        .list_packages("test-device", AppFilterType::All)
        .await
        .unwrap();
    assert_eq!(apps.len(), 3);
    assert!(apps[0].is_system);
    assert!(apps[1].is_system);
    assert!(!apps[2].is_system);
    let commands = server.await.unwrap();
    assert_eq!(commands[0], "sync:");
    assert!(commands[1].starts_with("shell:rm -f "));
    assert_eq!(commands[2], "shell:am get-current-user");
    assert_eq!(commands[3], "shell:pm list packages --user 10 -f");
    assert_eq!(commands[4], "shell:pm list packages --user 10 -s");
    assert_eq!(commands[5], "shell:dumpsys package");
}
