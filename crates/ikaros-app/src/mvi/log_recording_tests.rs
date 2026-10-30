use ikaros_domain::LogLevel;
use tokio::time::{timeout, Duration};

use super::*;

fn entry(number: usize) -> LogEntry {
    LogEntry {
        timestamp: "10-28 12:34:56.789".into(),
        pid: 123,
        tid: 456,
        package_name: Some("com.example.app".into()),
        process_name: Some("com.example.app".into()),
        level: LogLevel::Info,
        tag: "LongCapture".into(),
        message: format!("中文日志 {number}"),
    }
}

async fn update(receiver: &mut mpsc::UnboundedReceiver<AppIntent>) -> (bool, Option<String>) {
    match timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    {
        AppIntent::LogRecordingUpdated { enabled, error } => (enabled, error),
        other => panic!("Unexpected event: {other:?}"),
    }
}

async fn close(recorder: LogRecorder) {
    let mut signal = recorder.active_epoch.clone();
    drop(recorder);
    timeout(Duration::from_secs(5), async {
        while signal.changed().await.is_ok() {}
    })
    .await
    .unwrap();
}

async fn log_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut entries = fs::read_dir(directory).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        if entry.file_name().to_string_lossy().starts_with(FILE_PREFIX) {
            files.push(entry.path());
        }
    }
    files.sort();
    files
}

#[tokio::test]
async fn long_capture_survives_restart_and_off_cleans_only_generated_files() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("logs");
    let (tx, mut receiver) = mpsc::unbounded_channel();
    let recorder = LogRecorder::new(directory.clone(), tx.clone());
    assert_eq!(update(&mut receiver).await, (false, None));
    recorder.set_enabled(true).await.unwrap();
    assert_eq!(update(&mut receiver).await, (true, None));
    fs::write(directory.join("notes.txt"), "keep me")
        .await
        .unwrap();
    for start in (0..60_000).step_by(4096) {
        let entries: Vec<_> = (start..(start + 4096).min(60_000)).map(entry).collect();
        recorder
            .record_batch("device-a", 1, &entries)
            .await
            .unwrap();
    }
    close(recorder).await;
    let files = log_files(&directory).await;
    assert_eq!(files.len(), 1);
    let text = fs::read_to_string(&files[0]).await.unwrap();
    assert_eq!(text.lines().count(), 60_000);
    for (number, line) in text.lines().enumerate() {
        assert_eq!(
            line,
            format!(
                "10-28 12:34:56.789   123   456 I [com.example.app] LongCapture: 中文日志 {number}"
            )
        );
    }

    let restored = LogRecorder::new(directory.clone(), tx);
    assert_eq!(update(&mut receiver).await, (true, None));
    assert_eq!(fs::read_to_string(&files[0]).await.unwrap(), text);
    restored.set_enabled(false).await.unwrap();
    assert_eq!(update(&mut receiver).await, (false, None));
    restored
        .record_batch("device-a", 1, &[entry(60_000)])
        .await
        .unwrap();
    close(restored).await;
    assert!(log_files(&directory).await.is_empty());
    assert!(!directory.join(ENABLED_MARKER).exists());
    assert_eq!(
        fs::read_to_string(directory.join("notes.txt"))
            .await
            .unwrap(),
        "keep me"
    );
}

#[tokio::test]
async fn devices_and_sessions_are_separate_and_old_recording_epochs_are_rejected() {
    let temporary = tempfile::tempdir().unwrap();
    let (tx, mut receiver) = mpsc::unbounded_channel();
    let recorder = LogRecorder::new(temporary.path().to_path_buf(), tx);
    assert_eq!(update(&mut receiver).await, (false, None));
    recorder.set_enabled(true).await.unwrap();
    assert_eq!(update(&mut receiver).await, (true, None));
    let old_epoch = recorder.active_epoch.borrow().unwrap();
    recorder.record_batch("old", 0, &[entry(99)]).await.unwrap();
    recorder.set_enabled(false).await.unwrap();
    assert_eq!(update(&mut receiver).await, (false, None));
    recorder.set_enabled(true).await.unwrap();
    assert_eq!(update(&mut receiver).await, (true, None));
    recorder
        .commands
        .send(RecordingCommand::Append {
            epoch: old_epoch,
            serial: "stale".into(),
            generation: 0,
            entries: vec![entry(99)],
        })
        .await
        .unwrap();
    for (serial, generation, number) in [
        ("device/a:5555", 1, 1),
        ("device-b", 2, 2),
        ("device/a:5555", 3, 3),
    ] {
        recorder
            .record_batch(serial, generation, &[entry(number)])
            .await
            .unwrap();
    }
    close(recorder).await;
    let files = log_files(temporary.path()).await;
    assert_eq!(files.len(), 3);
    let mut numbers = Vec::new();
    for file in files {
        let text = fs::read_to_string(file).await.unwrap();
        assert_eq!(text.lines().count(), 1);
        numbers.push(
            text.split_whitespace()
                .last()
                .unwrap()
                .parse::<usize>()
                .unwrap(),
        );
    }
    numbers.sort();
    assert_eq!(numbers, [1, 2, 3]);
}

#[tokio::test]
async fn write_failure_disables_writes_and_keeps_existing_files() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("logs");
    let backup = temporary.path().join("saved");
    let (tx, mut receiver) = mpsc::unbounded_channel();
    let recorder = LogRecorder::new(directory.clone(), tx.clone());
    update(&mut receiver).await;
    recorder.set_enabled(true).await.unwrap();
    update(&mut receiver).await;
    recorder
        .record_batch("device", 1, &[entry(1)])
        .await
        .unwrap();
    close(recorder).await;
    let recorder = LogRecorder::new(directory.clone(), tx);
    assert_eq!(update(&mut receiver).await, (true, None));
    fs::rename(&directory, &backup).await.unwrap();
    fs::write(&directory, "blocked path").await.unwrap();
    recorder
        .record_batch("device", 2, &[entry(2)])
        .await
        .unwrap();
    let (enabled, error) = update(&mut receiver).await;
    assert!(
        enabled,
        "Keep the switch available for explicit cleanup after backing up"
    );
    assert!(error.is_some());
    assert_eq!(*recorder.active_epoch.borrow(), None);
    recorder
        .record_batch("device", 2, &[entry(3)])
        .await
        .unwrap();
    close(recorder).await;
    let files = log_files(&backup).await;
    assert_eq!(files.len(), 1);
    let text = fs::read_to_string(&files[0]).await.unwrap();
    assert!(text.ends_with("中文日志 1\n"));
    assert_eq!(text.lines().count(), 1);
    assert_eq!(fs::read_to_string(directory).await.unwrap(), "blocked path");
}
