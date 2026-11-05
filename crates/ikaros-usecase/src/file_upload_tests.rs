use super::*;
use async_trait::async_trait;
use ikaros_domain::{RemoteFile, SyncRepository};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingSync {
    directories: Mutex<Vec<String>>,
    uploads: Mutex<Vec<(String, Vec<u8>)>>,
    fail_on: Option<String>,
}

#[async_trait]
impl SyncRepository for RecordingSync {
    async fn list_directory(&self, _: &str, _: &str) -> Result<Vec<RemoteFile>, DomainError> {
        unreachable!()
    }

    async fn pull_file(
        &self,
        _: &str,
        _: &str,
        _: &Path,
        _: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        unreachable!()
    }

    async fn push_file(
        &self,
        serial: &str,
        local_path: &Path,
        remote_path: &str,
        progress_tx: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        assert_eq!(serial, "test-device");
        let bytes = tokio::fs::read(local_path).await.unwrap();
        let size = bytes.len() as u64;
        self.uploads
            .lock()
            .unwrap()
            .push((remote_path.into(), bytes));
        if self.fail_on.as_deref() == Some(remote_path) {
            return Err(DomainError::FileSyncFailed("permission denied".into()));
        }
        if let Some(tx) = progress_tx {
            for transferred_bytes in [0, size] {
                tx.send(TransferProgress {
                    filename: remote_path.into(),
                    transferred_bytes,
                    total_bytes: size,
                    speed_bytes_per_sec: 1024,
                    is_completed: transferred_bytes == size,
                })
                .unwrap();
                tokio::task::yield_now().await;
            }
        }
        Ok(())
    }

    async fn delete_path(&self, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }

    async fn rename_path(&self, _: &str, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }

    async fn make_directory(&self, serial: &str, path: &str) -> Result<(), DomainError> {
        assert_eq!(serial, "test-device");
        self.directories.lock().unwrap().push(path.into());
        Ok(())
    }
}

fn local_file(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

#[tokio::test]
async fn uploads_all_selected_files_with_monotonic_batch_progress() {
    let directory = tempfile::tempdir().unwrap();
    let paths = vec![
        local_file(directory.path(), "a.txt", b"hello"),
        local_file(directory.path(), "中文 ' 文件.txt", b"world!"),
    ];
    let repo = Arc::new(RecordingSync::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    FileUseCases::new(repo.clone())
        .push_paths("test-device", &paths, "/sdcard/目标/", Some(tx))
        .await
        .unwrap();

    assert_eq!(
        *repo.uploads.lock().unwrap(),
        vec![
            ("/sdcard/目标/a.txt".into(), b"hello".to_vec()),
            ("/sdcard/目标/中文 ' 文件.txt".into(), b"world!".to_vec()),
        ]
    );
    let mut updates = Vec::new();
    while let Ok(progress) = rx.try_recv() {
        updates.push(progress);
    }
    assert!(updates.iter().all(|progress| progress.total_bytes == 11));
    assert!(updates.windows(2).all(|pair| {
        pair[0].transferred_bytes <= pair[1].transferred_bytes && !pair[0].is_completed
    }));
    assert!(updates
        .iter()
        .any(|progress| progress.filename.starts_with("(1/2)")));
    assert!(updates
        .iter()
        .any(|progress| progress.filename.starts_with("(2/2)")));
    let completed = updates.last().unwrap();
    assert!(completed.is_completed);
    assert_eq!(completed.transferred_bytes, 11);
}

#[tokio::test]
async fn folder_upload_preserves_the_top_level_nested_and_empty_directories() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("资料");
    std::fs::create_dir_all(folder.join("nested/empty")).unwrap();
    local_file(&folder, ".hidden", b"hidden");
    local_file(&folder.join("nested"), "note.txt", b"nested");
    let repo = Arc::new(RecordingSync::default());
    FileUseCases::new(repo.clone())
        .push_paths("test-device", &[folder], "/sdcard", None)
        .await
        .unwrap();

    let directories: BTreeSet<_> = repo.directories.lock().unwrap().iter().cloned().collect();
    assert_eq!(
        directories,
        BTreeSet::from([
            "/sdcard/资料".into(),
            "/sdcard/资料/nested".into(),
            "/sdcard/资料/nested/empty".into(),
        ])
    );
    let uploads: BTreeMap<_, _> = repo.uploads.lock().unwrap().iter().cloned().collect();
    assert_eq!(
        uploads,
        BTreeMap::from([
            ("/sdcard/资料/.hidden".into(), b"hidden".to_vec()),
            ("/sdcard/资料/nested/note.txt".into(), b"nested".to_vec()),
        ])
    );
}

#[tokio::test]
async fn empty_folders_and_zero_byte_files_complete_successfully() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("empty");
    std::fs::create_dir(&folder).unwrap();
    let paths = vec![folder, local_file(directory.path(), "empty.txt", b"")];
    let repo = Arc::new(RecordingSync::default());
    let (tx, mut rx) = mpsc::unbounded_channel();
    FileUseCases::new(repo.clone())
        .push_paths("test-device", &paths, "/", Some(tx))
        .await
        .unwrap();

    assert_eq!(*repo.directories.lock().unwrap(), vec!["/empty"]);
    assert_eq!(
        *repo.uploads.lock().unwrap(),
        vec![("/empty.txt".into(), vec![])]
    );
    let mut completed = None;
    while let Ok(progress) = rx.try_recv() {
        completed = Some(progress);
    }
    let completed = completed.unwrap();
    assert!(completed.is_completed);
    assert_eq!(completed.total_bytes, 0);
    assert_eq!(completed.transferred_bytes, 0);
}

#[tokio::test]
async fn invalid_local_paths_fail_before_changing_the_device() {
    let directory = tempfile::tempdir().unwrap();
    let paths = vec![
        local_file(directory.path(), "valid.txt", b"valid"),
        directory.path().join("missing.txt"),
    ];
    let repo = Arc::new(RecordingSync::default());
    let error = FileUseCases::new(repo.clone())
        .push_paths("test-device", &paths, "/sdcard", None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing.txt"));
    assert!(repo.uploads.lock().unwrap().is_empty());
    assert!(repo.directories.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_upload_stops_the_remaining_batch_and_names_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let paths = vec![
        local_file(directory.path(), "denied.txt", b"first"),
        local_file(directory.path(), "remaining.txt", b"second"),
    ];
    let repo = Arc::new(RecordingSync {
        fail_on: Some("/sdcard/denied.txt".into()),
        ..Default::default()
    });
    let error = FileUseCases::new(repo.clone())
        .push_paths("test-device", &paths, "/sdcard", None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("/sdcard/denied.txt"));
    assert!(error.to_string().contains("permission denied"));
    assert_eq!(repo.uploads.lock().unwrap().len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_cycles_report_an_error_without_traversing_forever() {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("folder");
    std::fs::create_dir(&folder).unwrap();
    std::os::unix::fs::symlink(&folder, folder.join("cycle")).unwrap();
    let repo = Arc::new(RecordingSync::default());
    let error = FileUseCases::new(repo.clone())
        .push_paths("test-device", &[folder], "/sdcard", None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("cycle"));
    assert!(repo.uploads.lock().unwrap().is_empty());
    assert!(repo.directories.lock().unwrap().is_empty());
}
