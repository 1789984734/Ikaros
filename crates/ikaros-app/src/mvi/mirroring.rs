use super::AppStore;
use crate::{
    mvi::{state::MirroringProcess, AppIntent},
    scrcpy::{launch_scrcpy, ScrcpyLaunchOptions},
    scrcpy_recovery::{wait_for_hevc_stall, RecoveryStep},
};
use ikaros_domain::DeviceStatus;
use ikaros_usecase::DeviceUseCases;
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::{mpsc::UnboundedSender, watch},
};
use tracing::{debug, error, info, warn};

impl AppStore {
    pub(super) async fn start_mirroring(&mut self, serial: String, options: ScrcpyLaunchOptions) {
        if !self
            .state
            .devices
            .iter()
            .any(|device| device.serial == serial)
        {
            self.set_mirroring_status(&serial, "请选择已连接的设备再启动投屏".into());
            return;
        }
        if self.state.mirroring_processes.contains_key(&serial) {
            return;
        }
        self.launch_mirroring(serial, options, RecoveryStep::Initial, None)
            .await;
    }

    pub(super) async fn recover_mirroring(
        &mut self,
        serial: String,
        pid: u32,
        options: ScrcpyLaunchOptions,
        next: RecoveryStep,
    ) {
        if !self.remove_mirroring_process(&serial, pid) {
            return;
        }
        if !self
            .state
            .devices
            .iter()
            .any(|device| device.serial == serial && device.status == DeviceStatus::Device)
        {
            self.set_mirroring_status(&serial, format!("设备 [{serial}] 已断开，投屏已结束"));
            return;
        }
        self.launch_mirroring(serial, options, next, Some(pid))
            .await;
    }

    async fn launch_mirroring(
        &mut self,
        serial: String,
        options: ScrcpyLaunchOptions,
        step: RecoveryStep,
        previous_pid: Option<u32>,
    ) {
        let mut launch_options = options.clone();
        let using_h264_fallback = step == RecoveryStep::FallbackH264;
        if using_h264_fallback {
            launch_options.use_h264();
        }
        if let Some(pid) = previous_pid {
            launch_options.continue_recording(pid);
        }
        let child = match launch_scrcpy(&serial, &launch_options).await {
            Ok(child) => child,
            Err(error) => {
                self.set_mirroring_status(
                    &serial,
                    format!("设备 [{serial}] 启动投屏失败（{error}）"),
                );
                error!(%serial, %error, "启动 scrcpy 失败");
                return;
            }
        };
        let pid = child.id().expect("spawned scrcpy child has a process ID");
        let (stop_tx, stop_rx) = watch::channel(false);
        self.state.mirroring_processes.insert(
            serial.clone(),
            MirroringProcess {
                pid,
                using_h264_fallback,
            },
        );
        self.scrcpy_stop_txs.insert(pid, stop_tx);
        let status = match step {
            RecoveryStep::Initial => format!("投屏已启动（PID {pid}）"),
            RecoveryStep::RetryH265 => "检测到 H.265 编码中断，已重新启动 H.265 投屏".into(),
            RecoveryStep::FallbackH264 => "H.265 编码再次中断，已切换到 H.264".into(),
        };
        self.set_mirroring_status(&serial, format!("设备 [{serial}] {status}"));
        info!(%serial, pid, ?step, "scrcpy 启动成功");
        tokio::spawn(watch_scrcpy(
            child,
            MirroringRun {
                serial,
                pid,
                options,
                step,
            },
            stop_rx,
            self.device_usecases.clone(),
            self.intent_sender.clone(),
        ));
    }

    pub(super) fn stop_mirroring(&mut self, serial: String) {
        let Some(process) = self.state.mirroring_processes.remove(&serial) else {
            return;
        };
        if let Some(stop_tx) = self.scrcpy_stop_txs.remove(&process.pid) {
            let _ = stop_tx.send(true);
        }
        self.set_mirroring_status(&serial, format!("已停止设备 [{serial}] 的投屏"));
    }

    pub(super) fn scrcpy_exited(&mut self, serial: String, pid: u32) {
        if self.remove_mirroring_process(&serial, pid) {
            self.set_mirroring_status(&serial, format!("设备 [{serial}] 的投屏已结束"));
            info!(%serial, pid, "scrcpy 会话已结束");
        }
    }

    pub(super) fn scrcpy_recovery_blocked(&mut self, serial: &str, pid: u32) {
        if self.mirroring_pid(serial) == Some(pid) {
            self.set_mirroring_status(
                serial,
                format!("设备 [{serial}] H.265 编码中断，无法自动关闭当前投屏"),
            );
        }
    }

    fn set_mirroring_status(&mut self, serial: &str, message: String) {
        if self.state.current_serial() == serial {
            self.state.status_message = message;
        }
    }

    fn mirroring_pid(&self, serial: &str) -> Option<u32> {
        self.state
            .mirroring_processes
            .get(serial)
            .map(|process| process.pid)
    }

    fn remove_mirroring_process(&mut self, serial: &str, pid: u32) -> bool {
        if self.mirroring_pid(serial) != Some(pid) {
            return false;
        }
        self.state.mirroring_processes.remove(serial);
        self.scrcpy_stop_txs.remove(&pid);
        true
    }
}

struct MirroringRun {
    serial: String,
    pid: u32,
    options: ScrcpyLaunchOptions,
    step: RecoveryStep,
}

async fn watch_scrcpy(
    mut child: Child,
    run: MirroringRun,
    mut stop: watch::Receiver<bool>,
    devices: Arc<DeviceUseCases>,
    intents: UnboundedSender<AppIntent>,
) {
    let MirroringRun {
        serial,
        pid,
        options,
        step,
    } = run;
    if let Some(stderr) = child.stderr.take() {
        let device = serial.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(serial = %device, "scrcpy: {}", line);
            }
        });
    }
    let next = step.next().filter(|_| options.uses_h265_video());
    let restart = tokio::select! {
        biased;
        _ = stop.changed() => {
            info!(%serial, pid, "收到停止信号，终止 scrcpy 进程");
            let _ = child.kill().await;
            false
        }
        status = child.wait() => {
            if !status.as_ref().is_ok_and(|status| status.success()) {
                warn!(%serial, pid, ?status, "scrcpy 进程异常退出");
            }
            // scrcpy reports failure as 1 and a device disconnect as 2.
            step == RecoveryStep::RetryH265 && status.is_ok_and(|status| status.code() == Some(1))
        }
        () = wait_for_hevc_stall(&devices, &serial), if next.is_some() => {
            warn!(%serial, pid, ?step, "检测到 H.265 编码持续返回空帧");
            recover_stalled_process(&mut child, &options, &serial, pid, &mut stop, &intents).await
        }
    };
    let intent = match next.filter(|_| restart && !*stop.borrow()) {
        Some(next) => AppIntent::ScrcpyRecoveryRequested {
            serial,
            pid,
            options,
            next,
        },
        None => AppIntent::ScrcpyExited { serial, pid },
    };
    let _ = intents.send(intent);
}

async fn recover_stalled_process(
    child: &mut Child,
    options: &ScrcpyLaunchOptions,
    serial: &str,
    pid: u32,
    stop: &mut watch::Receiver<bool>,
    intents: &UnboundedSender<AppIntent>,
) -> bool {
    if !matches!(child.try_wait(), Ok(None)) {
        return false;
    }
    if !options.kills_adb_on_exit() && request_scrcpy_shutdown(child, pid).await {
        return true;
    }
    // Allow the recorder to finish the current file before opening a continuation.
    if options.recording_path().is_none() && child.kill().await.is_ok() {
        return true;
    }
    if !matches!(child.try_wait(), Ok(None)) {
        return false;
    }
    let _ = intents.send(AppIntent::ScrcpyRecoveryBlocked {
        serial: serial.to_owned(),
        pid,
    });
    tokio::select! {
        biased;
        _ = stop.changed() => { let _ = child.kill().await; }
        _ = child.wait() => {}
    }
    false
}

async fn request_scrcpy_shutdown(child: &mut Child, pid: u32) -> bool {
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("/bin/kill");
        command.args(["-TERM", &pid.to_string()]);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let Some(system_root) = std::env::var_os("SystemRoot") else {
            return false;
        };
        let taskkill = std::path::PathBuf::from(system_root).join("System32/taskkill.exe");
        let mut command = Command::new(taskkill);
        command
            .args(["/PID", &pid.to_string()])
            .creation_flags(0x08000000);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    tokio::time::timeout(Duration::from_secs(2), async {
        if command.status().await.is_ok_and(|status| status.success()) {
            child.wait().await.is_ok()
        } else {
            false
        }
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
#[path = "mirroring_tests.rs"]
mod tests;
