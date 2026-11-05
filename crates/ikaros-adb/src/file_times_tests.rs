use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

fn file(name: &str) -> RemoteFile {
    RemoteFile {
        name: name.into(),
        path: format!("/sdcard/{name}"),
        size: 1,
        is_dir: false,
        is_symlink: false,
        permissions: "644".into(),
        created_time: None,
        modified_time: 123,
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut length = [0; 4];
    stream.read_exact(&mut length).await.unwrap();
    let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await.unwrap();
    String::from_utf8(bytes).unwrap()
}

async fn fake_adb(outputs: Vec<String>) -> (AdbClient, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = AdbClient::new(listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut commands = Vec::new();
        for output in outputs {
            let (mut stream, _) = timeout(Duration::from_secs(3), listener.accept())
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
    (client, server)
}

#[tokio::test]
async fn unsupported_birth_time_stays_unknown_without_substituting_other_timestamps() {
    let (client, server) = fake_adb(vec!["?\r\n".into()]).await;
    let mut files = vec![file("a.txt"), file("b.txt")];
    client
        .load_creation_times("test-device", "/sdcard", &mut files)
        .await;
    assert!(files.iter().all(|file| file.created_time.is_none()));
    assert!(files.iter().all(|file| file.modified_time == 123));
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn supported_birth_times_are_read_in_batches_with_unknown_values_preserved() {
    let (client, server) = fake_adb(vec!["0\n".into(), "1793869868\r\n0\n-1\n?\n".into()]).await;
    let mut files = vec![
        file("a.txt"),
        file("b.txt"),
        file("c.txt"),
        file("quote ' and newline\n.txt"),
    ];
    client
        .load_creation_times("test-device", "/sdcard", &mut files)
        .await;
    assert_eq!(files[0].created_time, Some(1793869868));
    assert!(files[1..].iter().all(|file| file.created_time.is_none()));
    let commands = server.await.unwrap();
    assert_eq!(commands.len(), 2);
    assert!(commands[1].contains("'/sdcard/quote '\\'' and newline\n.txt'"));
    assert!(!commands[1].contains("%Z"));
}

#[tokio::test]
async fn large_directories_split_shell_requests_into_bounded_batches() {
    let (client, server) = fake_adb(vec![
        "0\n".into(),
        "1793869868\n".repeat(32),
        "1793869869\n".into(),
    ])
    .await;
    let mut files: Vec<_> = (0..33).map(|index| file(&format!("{index}.txt"))).collect();
    client
        .load_creation_times("test-device", "/sdcard", &mut files)
        .await;
    assert!(files[..32]
        .iter()
        .all(|file| file.created_time == Some(1793869868)));
    assert_eq!(files[32].created_time, Some(1793869869));
    assert_eq!(server.await.unwrap().len(), 3);
}

#[test]
fn incomplete_outputs_do_not_assign_another_files_creation_time() {
    let mut files = vec![file("a.txt"), file("b.txt")];
    apply_creation_times(&mut files, "1793869868\n");
    assert!(files.iter().all(|file| file.created_time.is_none()));
    apply_creation_times(&mut files, "0\n18446744073709551616\n");
    assert!(files.iter().all(|file| file.created_time.is_none()));
}

#[tokio::test]
async fn empty_directories_need_no_optional_metadata_requests() {
    let client = AdbClient::new("127.0.0.1:1".parse().unwrap());
    client
        .load_creation_times("test-device", "/sdcard", &mut [])
        .await;
}
