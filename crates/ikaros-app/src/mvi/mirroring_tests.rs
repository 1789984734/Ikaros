use super::*;
use ikaros_adb::AdbClient;
use ikaros_domain::{Device, DeviceStatus};
use ikaros_usecase::{
    AppUseCases, DeviceUseCases, FileUseCases, LogcatUseCases, PerformanceUseCases,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc::{self, UnboundedReceiver};

fn device(serial: &str) -> Device {
    Device {
        serial: serial.into(),
        status: DeviceStatus::Device,
        model: None,
        product: None,
        device: None,
        transport_id: None,
    }
}

fn store() -> (AppStore, UnboundedReceiver<AppIntent>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let adb = Arc::new(AdbClient::default());
    let (tx, rx) = mpsc::unbounded_channel();
    let mut store = AppStore::new(
        Arc::new(DeviceUseCases::new(adb.clone())),
        Arc::new(FileUseCases::new(adb.clone())),
        Arc::new(AppUseCases::new(adb.clone())),
        Arc::new(LogcatUseCases::new(adb.clone())),
        tx,
        directory.path().join("logs"),
        Arc::new(PerformanceUseCases::new(adb)),
    );
    store.state.devices = vec![device("first"), device("second")];
    (store, rx, directory)
}

fn options(path: &std::path::Path) -> ScrcpyLaunchOptions {
    ScrcpyLaunchOptions {
        binary_path: path.to_string_lossy().into_owned(),
        max_size: None,
        max_fps: None,
        bit_rate: None,
        video_codec: None,
        audio: false,
        stay_awake: false,
        extra_args: vec![],
    }
}

fn session(store: &mut AppStore, serial: &str, pid: u32) -> watch::Receiver<bool> {
    let (stop, signal) = watch::channel(false);
    store.state.mirroring_pids.insert(serial.into(), pid);
    store.scrcpy_stop_txs.insert(pid, stop);
    signal
}

#[tokio::test]
async fn switching_back_and_reselecting_reuses_existing_sessions() {
    let (mut store, _rx, directory) = store();
    let first = session(&mut store, "first", 101);
    let second = session(&mut store, "second", 202);
    store.state.active_tab = 1;
    let options = options(&directory.path().join("missing-scrcpy"));

    store.dispatch(AppIntent::SelectDevice(1)).await;
    assert!(store.state.is_mirroring());
    assert_eq!(store.state.current_serial(), "second");
    store.dispatch(AppIntent::StartMirroring(options)).await;
    store.dispatch(AppIntent::SelectDevice(0)).await;
    store.dispatch(AppIntent::SelectDevice(0)).await;

    assert!(store.state.is_mirroring());
    assert_eq!(store.state.mirroring_pids["first"], 101);
    assert_eq!(store.state.mirroring_pids["second"], 202);
    assert!(!first.has_changed().unwrap());
    assert!(!second.has_changed().unwrap());
}

#[tokio::test]
async fn stopping_the_selected_device_keeps_the_other_session_running() {
    let (mut store, _rx, _directory) = store();
    let first = session(&mut store, "first", 101);
    let second = session(&mut store, "second", 202);
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store.dispatch(AppIntent::StopMirroring).await;

    assert!(*second.borrow());
    assert!(!first.has_changed().unwrap());
    assert!(!store.state.is_mirroring());
    assert_eq!(store.state.mirroring_pids.len(), 1);
    store.dispatch(AppIntent::SelectDevice(0)).await;
    assert!(store.state.is_mirroring());
    assert_eq!(store.state.mirroring_pids["first"], 101);
}

#[tokio::test]
async fn background_and_stale_exit_events_do_not_clear_another_session() {
    let (mut store, _rx, _directory) = store();
    let _first = session(&mut store, "first", 101);
    let second = session(&mut store, "second", 202);
    store.state.selected_device_idx = 1;
    let status = store.state.status_message.clone();
    store
        .dispatch(AppIntent::ScrcpyExited {
            serial: "first".into(),
            pid: 101,
        })
        .await;
    assert!(store.state.is_mirroring());
    assert_eq!(store.state.status_message, status);

    let replacement = session(&mut store, "first", 303);
    for pid in [101, 202] {
        store
            .dispatch(AppIntent::ScrcpyExited {
                serial: "first".into(),
                pid,
            })
            .await;
    }
    assert_eq!(store.state.mirroring_pids["first"], 303);
    assert!(!replacement.has_changed().unwrap());
    assert!(!second.has_changed().unwrap());
    store
        .dispatch(AppIntent::ScrcpyExited {
            serial: "second".into(),
            pid: 202,
        })
        .await;
    assert!(!store.state.is_mirroring());
    assert_eq!(store.state.mirroring_pids["first"], 303);
}

#[tokio::test]
async fn failure_to_start_a_new_device_keeps_the_old_device_running() {
    let (mut store, _rx, directory) = store();
    let first = session(&mut store, "first", 101);
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store
        .dispatch(AppIntent::StartMirroring(options(
            &directory.path().join("missing-scrcpy"),
        )))
        .await;

    assert_eq!(store.state.current_serial(), "second");
    assert!(!store.state.is_mirroring());
    assert!(!store.state.is_busy);
    assert!(store.state.status_message.contains("启动投屏失败"));
    assert!(store.state.status_message.contains("second"));
    assert_eq!(store.state.mirroring_pids["first"], 101);
    assert!(!first.has_changed().unwrap());
}

#[tokio::test]
async fn device_list_reordering_preserves_selection_and_session_identity() {
    let (mut store, _rx, _directory) = store();
    let first = session(&mut store, "first", 101);
    let second = session(&mut store, "second", 202);
    store
        .dispatch(AppIntent::DevicesUpdated(vec![
            device("second"),
            device("first"),
        ]))
        .await;
    assert_eq!(store.state.current_serial(), "first");
    assert_eq!(store.state.selected_device_idx, 1);
    assert!(store.state.is_mirroring());

    store
        .dispatch(AppIntent::DevicesUpdated(vec![device("second")]))
        .await;
    assert_eq!(store.state.current_serial(), "second");
    assert!(store.state.is_mirroring());
    assert!(!first.has_changed().unwrap());
    assert!(!second.has_changed().unwrap());
    store.dispatch(AppIntent::DevicesUpdated(vec![])).await;
    assert!(!store.state.is_mirroring());
}

#[cfg(unix)]
async fn next_exit(intents: &mut UnboundedReceiver<AppIntent>) -> AppIntent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let intent = intents.recv().await.unwrap();
            if matches!(intent, AppIntent::ScrcpyExited { .. }) {
                return intent;
            }
        }
    })
    .await
    .unwrap()
}

#[cfg(unix)]
fn fake_scrcpy(directory: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let executable = directory.join("fake-scrcpy");
    std::fs::write(&executable, "#!/bin/sh\nexec /bin/sleep 10\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    executable
}

#[cfg(unix)]
#[tokio::test]
async fn switching_devices_never_starts_mirroring() {
    let (mut store, mut intents, directory) = store();
    let executable = fake_scrcpy(directory.path());
    store
        .dispatch(AppIntent::StartMirroring(options(&executable)))
        .await;
    let first_pid = store.state.mirroring_pids["first"];

    for tab in [0, 1] {
        store.dispatch(AppIntent::SwitchTab(tab)).await;
        store.dispatch(AppIntent::SelectDevice(1)).await;
        assert_eq!(store.state.current_serial(), "second");
        assert!(!store.state.is_mirroring());
        assert_eq!(store.state.mirroring_pids.len(), 1);
        assert_eq!(store.state.mirroring_pids["first"], first_pid);
        assert!(!*store.scrcpy_stop_txs[&first_pid].borrow());
        store.dispatch(AppIntent::SelectDevice(0)).await;
        assert!(store.state.is_mirroring());
        assert_eq!(store.state.mirroring_pids["first"], first_pid);
    }

    store.dispatch(AppIntent::StopMirroring).await;
    let exited = next_exit(&mut intents).await;
    store.dispatch(exited).await;
    for index in [1, 0] {
        store.dispatch(AppIntent::SelectDevice(index)).await;
        assert!(!store.state.is_mirroring());
        assert!(store.state.mirroring_pids.is_empty());
        assert!(store.scrcpy_stop_txs.is_empty());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn independent_processes_survive_switching_and_stop_individually() {
    let (mut store, mut intents, directory) = store();
    let executable = fake_scrcpy(directory.path());
    store
        .dispatch(AppIntent::StartMirroring(options(&executable)))
        .await;
    let first_pid = store.state.mirroring_pids["first"];
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store
        .dispatch(AppIntent::StartMirroring(options(&executable)))
        .await;
    let second_pid = store.state.mirroring_pids["second"];
    assert_ne!(first_pid, second_pid);
    for pid in [first_pid, second_pid] {
        assert!(std::process::Command::new("/bin/kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .unwrap()
            .success());
    }

    store.dispatch(AppIntent::SelectDevice(0)).await;
    assert_eq!(store.state.mirroring_pids["first"], first_pid);
    assert_eq!(store.state.mirroring_pids["second"], second_pid);
    store.dispatch(AppIntent::StopMirroring).await;
    let exited = next_exit(&mut intents).await;
    assert!(matches!(&exited, AppIntent::ScrcpyExited { serial, pid }
        if serial == "first" && *pid == first_pid));
    store.dispatch(exited).await;
    store.dispatch(AppIntent::SelectDevice(1)).await;
    assert!(store.state.is_mirroring());
    store.dispatch(AppIntent::StopMirroring).await;
    let exited = next_exit(&mut intents).await;
    assert!(matches!(&exited, AppIntent::ScrcpyExited { serial, pid }
        if serial == "second" && *pid == second_pid));
    store.dispatch(exited).await;
    assert!(store.state.mirroring_pids.is_empty());
    assert!(store.scrcpy_stop_txs.is_empty());
}
