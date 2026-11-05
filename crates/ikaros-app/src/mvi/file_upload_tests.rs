use super::*;
use async_trait::async_trait;
use ikaros_adb::AdbClient;
use ikaros_domain::{
    Device, DeviceStatus, DomainError, RemoteFile, SyncRepository, TransferProgress,
};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, Notify};

#[derive(Default)]
struct BlockingSync {
    started: Notify,
    attempts: AtomicUsize,
    active: AtomicBool,
}

struct ActiveUpload<'a>(&'a AtomicBool);

impl Drop for ActiveUpload<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

#[async_trait]
impl SyncRepository for BlockingSync {
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
        _: &str,
        _: &Path,
        _: &str,
        _: Option<UnboundedSender<TransferProgress>>,
    ) -> Result<(), DomainError> {
        self.attempts.fetch_add(1, Ordering::Relaxed);
        self.active.store(true, Ordering::Relaxed);
        let _active = ActiveUpload(&self.active);
        self.started.notify_one();
        std::future::pending().await
    }

    async fn delete_path(&self, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }

    async fn rename_path(&self, _: &str, _: &str, _: &str) -> Result<(), DomainError> {
        unreachable!()
    }

    async fn make_directory(&self, _: &str, _: &str) -> Result<(), DomainError> {
        Ok(())
    }
}

fn store(
    repo: Arc<BlockingSync>,
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

async fn next_completion(rx: &mut mpsc::UnboundedReceiver<AppIntent>) -> AppIntent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let intent = rx.recv().await.unwrap();
            if matches!(intent, AppIntent::UploadCompleted { .. }) {
                return intent;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn cancelling_a_batch_drops_the_active_upload_and_never_starts_the_remaining_files() {
    let directory = tempfile::tempdir().unwrap();
    let repo = Arc::new(BlockingSync::default());
    let (mut store, mut rx) = store(repo.clone(), directory.path());
    let paths = vec![
        directory.path().join("first"),
        directory.path().join("second"),
    ];
    for path in &paths {
        std::fs::write(path, b"contents").unwrap();
    }
    let intent = AppIntent::PushLocalPaths {
        local_paths: paths,
        remote_directory: "/sdcard".into(),
    };
    store.dispatch(intent.clone()).await;
    tokio::time::timeout(Duration::from_secs(3), repo.started.notified())
        .await
        .unwrap();
    let cancel = store.file_transfer_cancel.clone().unwrap();

    store.dispatch(intent).await;
    assert!(Arc::ptr_eq(
        store.file_transfer_cancel.as_ref().unwrap(),
        &cancel
    ));
    assert!(store.state.file_transfer_message.contains("已有传输进行中"));
    store.dispatch(AppIntent::CancelTransfer).await;
    let completion = next_completion(&mut rx).await;
    assert!(
        matches!(&completion, AppIntent::UploadCompleted { message, .. } if message.contains("已取消上传"))
    );
    store.dispatch(completion).await;

    assert!(!repo.active.load(Ordering::Relaxed));
    assert_eq!(repo.attempts.load(Ordering::Relaxed), 1);
    assert!(store.file_transfer_cancel.is_none());
    assert!(!store.state.is_busy);
    assert_eq!(store.state.file_transfer_percent, -1.0);
    assert!(store.state.status_message.contains("已取消上传"));
    assert!(
        matches!(rx.try_recv().unwrap(), AppIntent::BrowseDirectory(path) if path == "/sdcard")
    );
}

#[tokio::test]
async fn loading_a_directory_during_a_batch_keeps_progress_and_cancellation_available() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _rx) = store(Arc::new(BlockingSync::default()), directory.path());
    store.file_transfer_cancel = Some(Arc::new(AtomicBool::new(false)));
    store
        .dispatch(AppIntent::TransferProgressUpdated(TransferProgress {
            filename: "(2/3) photo.jpg".into(),
            transferred_bytes: 40,
            total_bytes: 100,
            speed_bytes_per_sec: 10,
            is_completed: false,
        }))
        .await;
    let message = store.state.file_transfer_message.clone();
    store
        .dispatch(AppIntent::DirectoryLoaded {
            serial: "test-device".into(),
            generation: 0,
            current_path: "/sdcard/elsewhere".into(),
            files: vec![],
        })
        .await;
    assert_eq!(store.state.current_dir, "/sdcard/elsewhere");
    assert_eq!(store.state.file_transfer_message, message);
    assert_eq!(store.state.file_transfer_percent, 0.4);
    assert!(store.state.is_busy);
    assert!(store.file_transfer_cancel.is_some());
}

#[tokio::test]
async fn zero_byte_batches_keep_the_cancel_control_visible_until_completion() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, _rx) = store(Arc::new(BlockingSync::default()), directory.path());
    for (is_completed, expected) in [(false, 0.0), (true, 1.0)] {
        store
            .dispatch(AppIntent::TransferProgressUpdated(TransferProgress {
                filename: "empty.txt".into(),
                transferred_bytes: 0,
                total_bytes: 0,
                speed_bytes_per_sec: 0,
                is_completed,
            }))
            .await;
        assert_eq!(store.state.file_transfer_percent, expected);
    }
}

#[tokio::test]
async fn upload_completion_from_another_device_does_not_refresh_the_current_device() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, mut rx) = store(Arc::new(BlockingSync::default()), directory.path());
    store.file_transfer_cancel = Some(Arc::new(AtomicBool::new(false)));
    store
        .dispatch(AppIntent::TransferProgressUpdated(TransferProgress {
            filename: "old-device.txt".into(),
            transferred_bytes: 10,
            total_bytes: 100,
            speed_bytes_per_sec: 10,
            is_completed: false,
        }))
        .await;
    let status = store.state.status_message.clone();
    store
        .dispatch(AppIntent::UploadCompleted {
            serial: "other-device".into(),
            message: "上传成功".into(),
        })
        .await;
    assert!(store.file_transfer_cancel.is_none());
    assert_eq!(store.state.status_message, status);
    assert!(store.state.file_transfer_message.is_empty());
    assert_eq!(store.state.file_transfer_percent, -1.0);
    while let Ok(intent) = rx.try_recv() {
        assert!(!matches!(intent, AppIntent::BrowseDirectory(_)));
    }
}
