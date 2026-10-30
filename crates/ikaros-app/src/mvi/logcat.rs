use std::sync::Arc;
use std::time::Duration;

use ikaros_usecase::LogcatUseCases;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval_at, Instant, MissedTickBehavior};

use super::log_recording::LogRecorder;
use super::AppIntent;

const BATCH_LIMIT: usize = 4096;
const BATCH_INTERVAL: Duration = Duration::from_millis(100);

/// Bound both input and pending batches; a slow consumer applies backpressure to ADB.
pub(super) async fn stream_log_batches(
    usecases: Arc<LogcatUseCases>,
    serial: String,
    generation: u64,
    intents: mpsc::UnboundedSender<AppIntent>,
    mut stop: watch::Receiver<bool>,
    recorder: Option<LogRecorder>,
) {
    let (log_tx, mut log_rx) = mpsc::channel(BATCH_LIMIT);
    let source = usecases.stream_logs(&serial, log_tx, stop.clone());
    tokio::pin!(source);
    let mut ticker = interval_at(Instant::now() + BATCH_INTERVAL, BATCH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut entries = Vec::with_capacity(BATCH_LIMIT);

    let result = loop {
        tokio::select! {
            biased;
            _ = stop.changed() => {
                // Preserve already received lines on pause/device switch; the UI rejects the old generation.
                entries.extend(std::iter::from_fn(|| log_rx.try_recv().ok()));
                save_batch(recorder.as_ref(), &serial, generation, &entries, &intents).await;
                return;
            },
            _ = ticker.tick(), if !entries.is_empty() => {
                let batch = std::mem::replace(&mut entries, Vec::with_capacity(BATCH_LIMIT));
                if !save_batch(recorder.as_ref(), &serial, generation, &batch, &intents).await {
                    return;
                }
                if intents.send(AppIntent::LogEntriesReceived { generation, entries: batch }).is_err() {
                    return;
                }
            }
            result = &mut source => break result,
            Some(entry) = log_rx.recv(), if entries.len() < BATCH_LIMIT => entries.push(entry),
        }
    };

    // EOF can arrive with a final partial batch and unread items in the bounded channel.
    entries.extend(std::iter::from_fn(|| log_rx.try_recv().ok()));
    if !save_batch(recorder.as_ref(), &serial, generation, &entries, &intents).await {
        return;
    }
    if !entries.is_empty() {
        let _ = intents.send(AppIntent::LogEntriesReceived {
            generation,
            entries,
        });
    }
    let _ = intents.send(AppIntent::LogcatStreamEnded {
        generation,
        error: result.err().map(|error| error.to_string()),
    });
}

async fn save_batch(
    recorder: Option<&LogRecorder>,
    serial: &str,
    generation: u64,
    entries: &[ikaros_domain::LogEntry],
    intents: &mpsc::UnboundedSender<AppIntent>,
) -> bool {
    if let Some(recorder) = recorder {
        if let Err(error) = recorder.record_batch(serial, generation, entries).await {
            let _ = intents.send(AppIntent::LogRecordingUpdated {
                enabled: true,
                error: Some(error),
            });
            return false;
        }
    }
    true
}

#[cfg(test)]
#[path = "logcat_tests.rs"]
mod tests;
