use super::AppStore;
use crate::{
    mvi::AppIntent,
    scrcpy::{launch_scrcpy, ScrcpyLaunchOptions},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Child,
    sync::{mpsc::UnboundedSender, watch},
};
use tracing::{debug, error, info, warn};

impl AppStore {
    pub(super) async fn start_mirroring(&mut self, options: ScrcpyLaunchOptions) {
        let Some(device) = self.state.current_device() else {
            self.state.status_message = "无法开启投屏: 未选择有效设备".into();
            return;
        };
        let serial = device.serial.clone();
        if self.state.is_mirroring() {
            return;
        }

        self.state.is_busy = true;
        self.state.status_message = format!("正在启动 scrcpy 投屏 [{}]...", serial);
        let result = launch_scrcpy(&serial, &options).await;
        self.state.is_busy = false;
        match result {
            Ok(child) => {
                let pid = child.id().unwrap_or(0);
                let (stop_tx, stop_rx) = watch::channel(false);
                self.state.mirroring_pids.insert(serial.clone(), pid);
                self.scrcpy_stop_txs.insert(pid, stop_tx);
                self.state.status_message = format!("设备 [{}] 的投屏已启动 (PID {})", serial, pid);
                info!(%serial, pid, "scrcpy 启动成功");
                tokio::spawn(watch_scrcpy(
                    child,
                    serial,
                    pid,
                    stop_rx,
                    self.intent_sender.clone(),
                ));
            }
            Err(error) => {
                self.state.status_message = format!("设备 [{}] 启动投屏失败: {}", serial, error);
                error!(%serial, %error, "启动 scrcpy 失败");
            }
        }
    }

    pub(super) fn stop_mirroring(&mut self) {
        let serial = self.state.current_serial();
        let Some(pid) = self.state.mirroring_pids.remove(&serial) else {
            return;
        };
        if let Some(stop_tx) = self.scrcpy_stop_txs.remove(&pid) {
            let _ = stop_tx.send(true);
        }
        self.state.status_message = format!("已停止设备 [{}] 的投屏", serial);
    }

    pub(super) fn scrcpy_exited(&mut self, serial: String, pid: u32) {
        if self.state.mirroring_pids.get(&serial) != Some(&pid) {
            return;
        }
        self.state.mirroring_pids.remove(&serial);
        self.scrcpy_stop_txs.remove(&pid);
        if serial == self.state.current_serial() {
            self.state.status_message = format!("设备 [{}] 的投屏已结束", serial);
        }
        info!(%serial, pid, "scrcpy 会话已结束");
    }
}

async fn watch_scrcpy(
    mut child: Child,
    serial: String,
    pid: u32,
    mut stop: watch::Receiver<bool>,
    intents: UnboundedSender<AppIntent>,
) {
    if let Some(stderr) = child.stderr.take() {
        let device = serial.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                debug!(serial = %device, "scrcpy: {}", line);
            }
        });
    }
    tokio::select! {
        _ = stop.changed() => {
            info!(%serial, pid, "收到停止信号，终止 scrcpy 进程");
            let _ = child.kill().await;
        }
        status = child.wait() => {
            match status {
                Ok(status) if status.success() => {},
                status => warn!(%serial, pid, ?status, "scrcpy 进程异常退出"),
            }
        }
    }
    let _ = intents.send(AppIntent::ScrcpyExited { serial, pid });
}

#[cfg(test)]
#[path = "mirroring_tests.rs"]
mod tests;
