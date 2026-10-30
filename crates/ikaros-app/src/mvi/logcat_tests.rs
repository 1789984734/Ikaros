use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use ikaros_domain::{DomainError, LogEntry, LogLevel, LogcatRepository};
use tokio::sync::Notify;
use tokio::time::{advance, timeout};

use super::*;

struct BurstSource {
    count: usize,
    emitted: AtomicUsize,
    filled: Notify,
}

impl BurstSource {
    fn new(count: usize) -> Arc<Self> {
        Arc::new(Self {
            count,
            emitted: AtomicUsize::new(0),
            filled: Notify::new(),
        })
    }
}

#[async_trait]
impl LogcatRepository for BurstSource {
    async fn start_logcat_stream(
        &self,
        _: &str,
        tx: mpsc::Sender<LogEntry>,
        _: watch::Receiver<bool>,
    ) -> Result<(), DomainError> {
        for number in 0..self.count {
            let entry = LogEntry {
                timestamp: String::new(),
                pid: 1,
                tid: 1,
                package_name: None,
                process_name: None,
                level: LogLevel::Info,
                tag: "Burst".into(),
                message: number.to_string(),
            };
            if tx.send(entry).await.is_err() {
                break;
            }
            if self.emitted.fetch_add(1, Ordering::SeqCst) + 1 == BATCH_LIMIT * 2 {
                self.filled.notify_one();
            }
        }
        Ok(())
    }

    async fn clear_logcat(&self, _: &str) -> Result<(), DomainError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn a_million_line_burst_is_bounded_and_can_be_stopped_while_full() {
    let source = BurstSource::new(1_000_000);
    let (intents, mut received) = mpsc::unbounded_channel();
    let (stop, signal) = watch::channel(false);
    let stream = tokio::spawn(stream_log_batches(
        Arc::new(LogcatUseCases::new(source.clone())),
        "test".into(),
        7,
        intents,
        signal,
        None,
    ));
    source.filled.notified().await;
    assert!(
        received.try_recv().is_err(),
        "Individual lines must not flood the intent queue"
    );
    assert_eq!(source.emitted.load(Ordering::SeqCst), BATCH_LIMIT * 2);

    advance(BATCH_INTERVAL).await;
    let AppIntent::LogEntriesReceived {
        generation,
        entries,
    } = received.recv().await.unwrap()
    else {
        panic!("Expected a batch");
    };
    assert_eq!(generation, 7);
    assert_eq!(entries.len(), BATCH_LIMIT);
    stop.send(true).unwrap();
    timeout(Duration::from_secs(1), stream)
        .await
        .unwrap()
        .unwrap();
    assert!(source.emitted.load(Ordering::SeqCst) <= BATCH_LIMIT * 3);
    assert!(
        received.try_recv().is_err(),
        "Stopping must discard a pending stale batch"
    );
}

#[tokio::test(start_paused = true)]
async fn end_of_stream_flushes_every_line_in_order_including_the_partial_batch() {
    let count = BATCH_LIMIT * 3 + 17;
    let source = BurstSource::new(count);
    let (intents, mut received) = mpsc::unbounded_channel();
    let (_stop, signal) = watch::channel(false);
    let stream = tokio::spawn(stream_log_batches(
        Arc::new(LogcatUseCases::new(source)),
        "test".into(),
        3,
        intents,
        signal,
        None,
    ));
    let mut next = 0;
    loop {
        match received.recv().await.unwrap() {
            AppIntent::LogEntriesReceived {
                generation,
                entries,
            } => {
                assert_eq!(generation, 3);
                assert!(entries.len() <= BATCH_LIMIT * 2);
                for entry in entries {
                    assert_eq!(entry.message, next.to_string());
                    next += 1;
                }
            }
            AppIntent::LogcatStreamEnded { generation, error } => {
                assert_eq!(generation, 3);
                assert!(error.is_none());
                break;
            }
            other => panic!("Unexpected intent: {other:?}"),
        }
    }
    assert_eq!(next, count);
    stream.await.unwrap();
}

#[tokio::test]
async fn pausing_capture_saves_the_pending_batch_and_received_channel_tail() {
    let temporary = tempfile::tempdir().unwrap();
    let (intents, mut received) = mpsc::unbounded_channel();
    let recorder = LogRecorder::new(temporary.path().to_path_buf(), intents.clone());
    assert!(matches!(
        received.recv().await,
        Some(AppIntent::LogRecordingUpdated { error: None, .. })
    ));
    recorder.set_enabled(true).await.unwrap();
    assert!(matches!(
        received.recv().await,
        Some(AppIntent::LogRecordingUpdated {
            enabled: true,
            error: None
        })
    ));
    let source = BurstSource::new(1_000_000);
    let (stop, signal) = watch::channel(false);
    let stream = tokio::spawn(stream_log_batches(
        Arc::new(LogcatUseCases::new(source.clone())),
        "test".into(),
        1,
        intents,
        signal,
        Some(recorder.clone()),
    ));
    timeout(Duration::from_secs(5), source.filled.notified())
        .await
        .unwrap();
    stop.send(true).unwrap();
    timeout(Duration::from_secs(5), stream)
        .await
        .unwrap()
        .unwrap();
    let count = source.emitted.load(Ordering::SeqCst);
    timeout(Duration::from_secs(5), async {
        loop {
            let mut directory = tokio::fs::read_dir(temporary.path()).await.unwrap();
            while let Some(file) = directory.next_entry().await.unwrap() {
                if file
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "log")
                {
                    let text = tokio::fs::read_to_string(file.path()).await.unwrap();
                    if text.lines().count() == count {
                        for (number, line) in text.lines().enumerate() {
                            assert!(line.ends_with(&format!("Burst: {number}")));
                        }
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    recorder.set_enabled(false).await.unwrap();
    assert!(matches!(
        received.recv().await,
        Some(AppIntent::LogRecordingUpdated {
            enabled: false,
            error: None
        })
    ));
}
