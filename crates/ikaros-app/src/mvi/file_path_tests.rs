use super::*;
use async_trait::async_trait;
use ikaros_adb::AdbClient;
use ikaros_domain::{
    Device, DeviceStatus, DomainError, RemoteFile, SyncRepository, TransferProgress,
};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

#[derive(Default)]
struct DirectorySync {
    calls: Mutex<Vec<String>>,
    started: Notify,
}

fn file(parent: &str, name: &str, is_dir: bool) -> RemoteFile {
    RemoteFile {
        name: name.into(),
        path: format!("{parent}/{name}"),
        size: 1,
        is_dir,
        is_symlink: false,
        permissions: "755".into(),
        created_time: None,
        modified_time: 123,
    }
}

#[async_trait]
impl SyncRepository for DirectorySync {
    async fn list_directory(&self, _: &str, path: &str) -> Result<Vec<RemoteFile>, DomainError> {
        self.calls.lock().unwrap().push(path.into());
        self.started.notify_one();
        match path {
            "/slow" => std::future::pending().await,
            "/sdcard/missing" => Err(DomainError::FileSyncFailed("目录不存在".into())),
            _ => Ok(vec![
                file(path, "Download.txt", false),
                file(path, "Download", true),
                file(path, "Documents", true),
                file(path, "中文 资料", true),
            ]),
        }
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
        _: &str,
        _: &Path,
        _: &str,
        _: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        unreachable!()
    }
    async fn delete_path(&self, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }
    async fn rename_path(&self, _: &str, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }
    async fn make_directory(&self, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }
}

fn store(
    repo: Arc<DirectorySync>,
    directory: &Path,
) -> (AppStore, mpsc::UnboundedReceiver<AppIntent>) {
    let adb = Arc::new(AdbClient::new("127.0.0.1:1".parse().unwrap()));
    let (tx, rx) = mpsc::unbounded_channel();
    let mut store = AppStore::new(
        Arc::new(DeviceUseCases::new(adb.clone())),
        Arc::new(FileUseCases::new(repo)),
        Arc::new(AppUseCases::new(adb.clone())),
        Arc::new(LogcatUseCases::new(adb.clone())),
        tx,
        directory.join("logs"),
        Arc::new(PerformanceUseCases::new(adb)),
    );
    store.state.devices = vec![Device {
        serial: "test-device".into(),
        status: DeviceStatus::Device,
        model: None,
        product: None,
        device: None,
        transport_id: None,
    }];
    (store, rx)
}

async fn next_file_result(rx: &mut mpsc::UnboundedReceiver<AppIntent>) -> AppIntent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let intent = rx.recv().await.unwrap();
            if matches!(
                intent,
                AppIntent::FilePathCompletionsLoaded { .. }
                    | AppIntent::DirectoryLoaded { .. }
                    | AppIntent::DirectoryLoadFailed { .. }
            ) {
                return intent;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn completion_debounces_typing_and_only_suggests_matching_directories() {
    let directory = tempfile::tempdir().unwrap();
    let repo = Arc::new(DirectorySync::default());
    let (mut store, mut rx) = store(repo.clone(), directory.path());
    store
        .dispatch(AppIntent::CompleteFilePath("/sdcard/D".into()))
        .await;
    store
        .dispatch(AppIntent::CompleteFilePath("/sdcard/Down".into()))
        .await;
    let result = next_file_result(&mut rx).await;
    store.dispatch(result).await;
    assert_eq!(*repo.calls.lock().unwrap(), vec!["/sdcard"]);
    assert_eq!(store.state.file_path_suggestions, vec!["/sdcard/Download/"]);
    assert_eq!(store.state.current_dir, "/sdcard");
    assert!(!store.state.is_busy);

    store
        .dispatch(AppIntent::CompleteFilePath("中文".into()))
        .await;
    let result = next_file_result(&mut rx).await;
    store.dispatch(result).await;
    assert_eq!(
        store.state.file_path_suggestions,
        vec!["/sdcard/中文 资料/"]
    );
}

#[tokio::test]
async fn old_completions_are_rejected_after_editing_or_switching_devices() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _rx) = store(Arc::new(DirectorySync::default()), directory.path());
    store
        .dispatch(AppIntent::CompleteFilePath("/sdcard/D".into()))
        .await;
    let old = store.file_completion_generation;
    store
        .dispatch(AppIntent::CompleteFilePath("/sdcard/M".into()))
        .await;
    store
        .dispatch(AppIntent::FilePathCompletionsLoaded {
            serial: "test-device".into(),
            generation: old,
            choices: vec!["/sdcard/Download/".into()],
        })
        .await;
    assert!(store.state.file_path_suggestions.is_empty());
    let generation = store.file_completion_generation;
    let mut other = store.state.devices[0].clone();
    other.serial = "other-device".into();
    store.state.devices.push(other);
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store
        .dispatch(AppIntent::FilePathCompletionsLoaded {
            serial: "test-device".into(),
            generation,
            choices: vec!["/sdcard/Music/".into()],
        })
        .await;
    assert!(store.state.file_path_suggestions.is_empty());
    assert!(store.state.file_path_completion_query.is_empty());
    assert!(store.file_completion_task.is_none());
}

#[tokio::test]
async fn failed_relative_navigation_keeps_the_last_successful_directory_and_files() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, mut rx) = store(Arc::new(DirectorySync::default()), directory.path());
    store.state.current_dir = "/sdcard/Music".into();
    store.state.file_list = vec![file("/sdcard/Music", "song.flac", false)];
    store
        .dispatch(AppIntent::BrowseDirectory("../missing".into()))
        .await;
    let result = next_file_result(&mut rx).await;
    assert!(matches!(result, AppIntent::DirectoryLoadFailed { .. }));
    store.dispatch(result).await;
    assert_eq!(store.state.current_dir, "/sdcard/Music");
    assert_eq!(store.state.file_list[0].name, "song.flac");
    assert!(store.state.file_transfer_message.contains("目录不存在"));
    assert!(!store.state.is_busy);
}

#[tokio::test]
async fn new_navigation_replaces_slow_requests_and_ignores_queued_old_results() {
    let directory = tempfile::tempdir().unwrap();
    let repo = Arc::new(DirectorySync::default());
    let (mut store, mut rx) = store(repo.clone(), directory.path());
    store
        .dispatch(AppIntent::BrowseDirectory("/slow".into()))
        .await;
    tokio::time::timeout(Duration::from_secs(3), repo.started.notified())
        .await
        .unwrap();
    let generation = store.file_browse_generation;
    store
        .dispatch(AppIntent::BrowseDirectory("/sdcard/Download".into()))
        .await;
    let result = next_file_result(&mut rx).await;
    store.dispatch(result).await;
    assert_eq!(store.state.current_dir, "/sdcard/Download");
    store
        .dispatch(AppIntent::DirectoryLoaded {
            serial: "test-device".into(),
            generation,
            current_path: "/slow".into(),
            files: vec![],
        })
        .await;
    assert_eq!(store.state.current_dir, "/sdcard/Download");
    assert!(!store.state.file_list.is_empty());
}
