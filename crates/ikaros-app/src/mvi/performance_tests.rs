use super::*;
use ikaros_adb::AdbClient;
use ikaros_domain::{Device, DeviceStatus, DomainError, PerformanceReading, PerformanceRepository};
use ikaros_usecase::{AppUseCases, DeviceUseCases, FileUseCases, LogcatUseCases};
use std::sync::atomic::{AtomicUsize, Ordering};

struct PendingPerformance(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl PerformanceRepository for PendingPerformance {
    async fn read_performance(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<PerformanceReading, DomainError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
    async fn performance_packages(&self, _: &str) -> Result<Vec<String>, DomainError> {
        Ok(vec!["com.example".into()])
    }
}

fn store() -> (AppStore, Arc<AtomicUsize>) {
    let adb = Arc::new(AdbClient::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut store = AppStore::new(
        Arc::new(DeviceUseCases::new(adb.clone())),
        Arc::new(FileUseCases::new(adb.clone())),
        Arc::new(AppUseCases::new(adb.clone())),
        Arc::new(LogcatUseCases::new(adb)),
        tx,
        tempfile::tempdir().unwrap().path().join("logs"),
        Arc::new(PerformanceUseCases::new(Arc::new(PendingPerformance(
            calls.clone(),
        )))),
    );
    store.state.devices = vec![Device {
        serial: "test-1".into(),
        status: DeviceStatus::Device,
        model: None,
        product: None,
        device: None,
        transport_id: None,
    }];
    (store, calls)
}

#[tokio::test]
async fn history_is_bounded_and_pause_or_target_switch_rejects_stale_samples() {
    let (mut store, _) = store();
    store.state.active_tab = 6;
    store.start_performance();
    let generation = store.performance_generation;
    for i in 0..200 {
        store.receive_performance_sample(
            generation,
            PerformanceSample {
                elapsed_seconds: i as f64,
                ..Default::default()
            },
        );
    }
    assert_eq!(store.state.performance.history.len(), HISTORY_LIMIT);
    assert_eq!(
        store
            .state
            .performance
            .history
            .front()
            .unwrap()
            .elapsed_seconds,
        80.0
    );
    store.toggle_performance();
    store.receive_performance_sample(generation, PerformanceSample::default());
    assert_eq!(store.state.performance.history.len(), HISTORY_LIMIT);
    store.select_performance_package("com.example".into());
    store.receive_performance_sample(generation, PerformanceSample::default());
    assert!(store.state.performance.history.is_empty());
    assert!(store.state.performance.is_running);
}

#[tokio::test(start_paused = true)]
async fn slow_sampling_is_cancelled_on_leaving_and_does_not_overlap() {
    let (mut store, calls) = store();
    store.dispatch(AppIntent::SwitchTab(6)).await;
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(5)).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    store.dispatch(AppIntent::SwitchTab(0)).await;
    assert!(!store.state.performance.is_running);
    assert!(store.performance_task.is_none());
    tokio::time::advance(Duration::from_secs(20)).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn disconnect_resets_target_and_old_device_results_are_ignored() {
    let (mut store, _) = store();
    store.state.active_tab = 6;
    store.select_performance_package("com.example".into());
    let generation = store.performance_generation;
    store.dispatch(AppIntent::DevicesUpdated(vec![])).await;
    assert!(!store.state.performance.is_running);
    assert!(store.state.performance.package.is_empty());
    store.receive_performance_sample(generation, PerformanceSample::default());
    store
        .dispatch(AppIntent::PerformanceFailed {
            generation,
            error: "old failure".into(),
        })
        .await;
    store
        .dispatch(AppIntent::PerformancePackagesLoaded {
            serial: "test-1".into(),
            result: Ok(vec!["stale".into()]),
        })
        .await;
    assert!(store.state.performance.history.is_empty());
    assert!(store.state.performance.packages.is_empty());
    assert!(!store.state.performance.has_error);
}
