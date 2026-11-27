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
    store_with_client(AdbClient::default())
}

fn store_with_client(
    adb: AdbClient,
) -> (AppStore, UnboundedReceiver<AppIntent>, tempfile::TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let adb = Arc::new(adb);
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
    store.state.mirroring_processes.insert(
        serial.into(),
        MirroringProcess {
            pid,
            using_h264_fallback: false,
        },
    );
    store.scrcpy_stop_txs.insert(pid, stop);
    signal
}

fn start(serial: &str, options: ScrcpyLaunchOptions) -> AppIntent {
    AppIntent::StartMirroring {
        serial: serial.into(),
        options,
    }
}

fn stop(serial: &str) -> AppIntent {
    AppIntent::StopMirroring {
        serial: serial.into(),
    }
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
    store.dispatch(start("second", options)).await;
    store.dispatch(AppIntent::SelectDevice(0)).await;
    store.dispatch(AppIntent::SelectDevice(0)).await;

    assert!(store.state.is_mirroring());
    assert_eq!(store.mirroring_pid("first"), Some(101));
    assert_eq!(store.mirroring_pid("second"), Some(202));
    assert!(!first.has_changed().unwrap());
    assert!(!second.has_changed().unwrap());
}

#[tokio::test]
async fn stopping_the_selected_device_keeps_the_other_session_running() {
    let (mut store, _rx, _directory) = store();
    let first = session(&mut store, "first", 101);
    let second = session(&mut store, "second", 202);
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store.dispatch(stop("second")).await;

    assert!(*second.borrow());
    assert!(!first.has_changed().unwrap());
    assert!(!store.state.is_mirroring());
    assert_eq!(store.state.mirroring_processes.len(), 1);
    store.dispatch(AppIntent::SelectDevice(0)).await;
    assert!(store.state.is_mirroring());
    assert_eq!(store.mirroring_pid("first"), Some(101));
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
    assert_eq!(store.mirroring_pid("first"), Some(303));
    assert!(!replacement.has_changed().unwrap());
    assert!(!second.has_changed().unwrap());
    store
        .dispatch(AppIntent::ScrcpyExited {
            serial: "second".into(),
            pid: 202,
        })
        .await;
    assert!(!store.state.is_mirroring());
    assert_eq!(store.mirroring_pid("first"), Some(303));
}

#[tokio::test]
async fn failure_to_start_a_new_device_keeps_the_old_device_running() {
    let (mut store, _rx, directory) = store();
    let first = session(&mut store, "first", 101);
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store
        .dispatch(start(
            "second",
            options(&directory.path().join("missing-scrcpy")),
        ))
        .await;

    assert_eq!(store.state.current_serial(), "second");
    assert!(!store.state.is_mirroring());
    assert!(!store.state.is_busy);
    assert!(store.state.status_message.contains("启动投屏失败"));
    assert!(store.state.status_message.contains("second"));
    assert_eq!(store.mirroring_pid("first"), Some(101));
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
    store.dispatch(start("first", options(&executable))).await;
    let first_pid = store.mirroring_pid("first").unwrap();

    for tab in [0, 1] {
        store.dispatch(AppIntent::SwitchTab(tab)).await;
        store.dispatch(AppIntent::SelectDevice(1)).await;
        assert_eq!(store.state.current_serial(), "second");
        assert!(!store.state.is_mirroring());
        assert_eq!(store.state.mirroring_processes.len(), 1);
        assert_eq!(store.mirroring_pid("first"), Some(first_pid));
        assert!(!*store.scrcpy_stop_txs[&first_pid].borrow());
        store.dispatch(AppIntent::SelectDevice(0)).await;
        assert!(store.state.is_mirroring());
        assert_eq!(store.mirroring_pid("first"), Some(first_pid));
    }

    store.dispatch(stop("first")).await;
    let exited = next_exit(&mut intents).await;
    store.dispatch(exited).await;
    for index in [1, 0] {
        store.dispatch(AppIntent::SelectDevice(index)).await;
        assert!(!store.state.is_mirroring());
        assert!(store.state.mirroring_processes.is_empty());
        assert!(store.scrcpy_stop_txs.is_empty());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn independent_processes_survive_switching_and_stop_individually() {
    let (mut store, mut intents, directory) = store();
    let executable = fake_scrcpy(directory.path());
    store.dispatch(start("first", options(&executable))).await;
    let first_pid = store.mirroring_pid("first").unwrap();
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store.dispatch(start("second", options(&executable))).await;
    let second_pid = store.mirroring_pid("second").unwrap();
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
    assert_eq!(store.mirroring_pid("first"), Some(first_pid));
    assert_eq!(store.mirroring_pid("second"), Some(second_pid));
    store.dispatch(stop("first")).await;
    let exited = next_exit(&mut intents).await;
    assert!(matches!(&exited, AppIntent::ScrcpyExited { serial, pid }
        if serial == "first" && *pid == first_pid));
    store.dispatch(exited).await;
    store.dispatch(AppIntent::SelectDevice(1)).await;
    assert!(store.state.is_mirroring());
    store.dispatch(stop("second")).await;
    let exited = next_exit(&mut intents).await;
    assert!(matches!(&exited, AppIntent::ScrcpyExited { serial, pid }
        if serial == "second" && *pid == second_pid));
    store.dispatch(exited).await;
    assert!(store.state.mirroring_processes.is_empty());
    assert!(store.scrcpy_stop_txs.is_empty());
}

#[cfg(unix)]
async fn send_hevc_fault(listener: &tokio::net::TcpListener, serial: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut header = [0; 4];
        socket.read_exact(&mut header).await.unwrap();
        let length = usize::from_str_radix(std::str::from_utf8(&header).unwrap(), 16).unwrap();
        let mut request = vec![0; length];
        socket.read_exact(&mut request).await.unwrap();
        String::from_utf8(request).unwrap()
    }

    tokio::time::timeout(Duration::from_secs(5), async {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert_eq!(read_request(&mut socket).await, format!("host:transport:{serial}"));
        socket.write_all(b"OKAY").await.unwrap();
        let command = read_request(&mut socket).await;
        assert!(command.starts_with("shell:logcat "));
        assert!(command.contains("-T 1"));
        socket.write_all(b"OKAY").await.unwrap();
        let failed_frame = concat!(
            "11-26 10:57:02.085 401 6097 E VSP : H265Enc_WriteSlice, 564, BSM or IOMMU error\n",
            "11-26 10:57:02.112 401 6097 I SPRDHEVCEncoder: H265EncStrmEncode[300] 31ms, in {0xe6898000-0x10229000, 448x592, 5}, out {0xead39000-0, 0}, wh{0, 0}\n",
        );
        socket.write_all(failed_frame.repeat(5).as_bytes()).await.unwrap();
        assert_eq!(socket.read(&mut [0_u8; 1]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[cfg(unix)]
async fn next_recovery(intents: &mut UnboundedReceiver<AppIntent>) -> AppIntent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let intent = intents.recv().await.unwrap();
            if matches!(intent, AppIntent::ScrcpyRecoveryRequested { .. }) {
                return intent;
            }
        }
    })
    .await
    .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn confirmed_hevc_faults_restart_then_fall_back_on_the_same_device() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adb = AdbClient::new(listener.local_addr().unwrap());
    let (mut store, mut intents, directory) = store_with_client(adb);
    let executable = fake_scrcpy(directory.path());
    let mut hevc = options(&executable);
    hevc.video_codec = Some("h265".into());
    store.dispatch(start("first", hevc)).await;
    let initial_pid = store.mirroring_pid("first").unwrap();
    store.dispatch(AppIntent::SelectDevice(1)).await;
    store.dispatch(start("second", options(&executable))).await;
    let second_pid = store.mirroring_pid("second");

    send_hevc_fault(&listener, "first").await;
    let retry = next_recovery(&mut intents).await;
    assert!(matches!(
        retry,
        AppIntent::ScrcpyRecoveryRequested {
            next: RecoveryStep::RetryH265,
            ..
        }
    ));
    store.dispatch(retry).await;
    let retried_pid = store.mirroring_pid("first").unwrap();
    assert_ne!(initial_pid, retried_pid);
    assert_eq!(store.mirroring_pid("second"), second_pid);

    send_hevc_fault(&listener, "first").await;
    let fallback = next_recovery(&mut intents).await;
    assert!(matches!(
        fallback,
        AppIntent::ScrcpyRecoveryRequested {
            next: RecoveryStep::FallbackH264,
            ..
        }
    ));
    store.dispatch(fallback).await;
    assert_ne!(store.mirroring_pid("first"), Some(retried_pid));
    assert_eq!(store.mirroring_pid("second"), second_pid);
    assert!(!store.state.is_using_h264_fallback());
    store.dispatch(AppIntent::SelectDevice(0)).await;
    assert!(store.state.is_using_h264_fallback());

    store.dispatch(stop("first")).await;
    store.dispatch(stop("second")).await;
    for _ in 0..2 {
        store.dispatch(next_exit(&mut intents).await).await;
    }
    assert!(store.state.mirroring_processes.is_empty());
}

#[tokio::test]
async fn queued_recovery_cannot_reopen_a_device_after_manual_stop() {
    let (mut store, _intents, directory) = store();
    let running = session(&mut store, "first", 101);
    store.dispatch(stop("first")).await;
    store
        .dispatch(AppIntent::ScrcpyRecoveryRequested {
            serial: "first".into(),
            pid: 101,
            options: options(&directory.path().join("missing-scrcpy")),
            next: RecoveryStep::RetryH265,
        })
        .await;
    assert!(*running.borrow());
    assert!(!store.state.is_mirroring());
    assert!(store.state.status_message.contains("已停止"));
}
