use super::*;
use ikaros_domain::SyncRepository;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[cfg(unix)]
#[test]
fn directory_completion_includes_hidden_and_linked_folders_and_preserves_special_names() {
    use std::{collections::BTreeSet, process::Command};
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("quote ' $(printf unsafe)");
    std::fs::create_dir(&parent).unwrap();
    for name in ["Download", ".hidden", "..hidden", "中文 目录\n"] {
        std::fs::create_dir(parent.join(name)).unwrap();
    }
    std::fs::write(parent.join("file.txt"), b"file").unwrap();
    std::os::unix::fs::symlink(parent.join("Download"), parent.join("linked")).unwrap();
    let output = Command::new("sh")
        .arg("-c")
        .arg(subdirectories_command(parent.to_str().unwrap()))
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = String::from_utf8(output.stdout).unwrap();
    let actual: BTreeSet<_> = output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| path.rsplit('/').next().unwrap())
        .collect();
    assert_eq!(
        actual,
        BTreeSet::from(["Download", ".hidden", "..hidden", "中文 目录\n", "linked"])
    );
}

#[tokio::test]
async fn nonexistent_directory_is_reported_instead_of_being_shown_as_empty() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = AdbClient::new(listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        for expected in ["host:transport:test-device", "shell:if "] {
            let mut length = [0; 4];
            stream.read_exact(&mut length).await.unwrap();
            let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).await.unwrap();
            assert!(String::from_utf8(bytes).unwrap().starts_with(expected));
            stream.write_all(b"OKAY").await.unwrap();
        }
    });
    let error = client
        .list_directory("test-device", "/missing")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("目录不存在或无权访问: /missing"));
    server.await.unwrap();
}
