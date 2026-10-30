use super::{AppIntent, AppStore};
use std::sync::Arc;

impl AppStore {
    /// 流式执行终端命令：输出实时回传，可随时中断（断开即杀死远端进程）
    pub(super) fn on_execute_shell(&mut self, cmd: String) {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            return;
        }
        let serial = self.state.current_serial();
        if serial == "未连接" {
            self.state
                .shell_output
                .push_str("错误: 当前没有可用的连接设备\n");
            return;
        }
        if self.state.is_shell_busy {
            self.state.status_message = "已有命令正在执行，请先中断或等待完成".into();
            return;
        }

        self.state.is_busy = true;
        self.state.is_shell_busy = true;
        self.state.status_message = format!("执行命令: {}", cmd);
        self.state.shell_output.push_str(&format!("$ {}\n", cmd));

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        self.shell_stop_tx = Some(stop_tx);
        self.shell_generation = self.shell_generation.wrapping_add(1);
        let generation = self.shell_generation;

        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::unbounded_channel();
            let tx_chunk = tx.clone();
            let stream_task = tokio::spawn(async move {
                usecases
                    .stream_shell(&serial, &cmd, chunk_tx, stop_rx)
                    .await
            });
            while let Some(chunk) = chunk_rx.recv().await {
                let _ = tx_chunk.send(AppIntent::ShellOutputReceived(chunk));
            }
            let error = match stream_task.await {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(e.to_string()),
                Err(e) => Some(format!("命令执行任务中断: {e}")),
            };
            let _ = tx.send(AppIntent::ShellCommandFinished { generation, error });
        });
    }

    pub(super) fn on_interrupt_shell(&mut self) {
        if let Some(stop_tx) = self.shell_stop_tx.take() {
            let _ = stop_tx.send(true);
        }
    }

    pub(super) fn on_shell_output_received(&mut self, output: String) {
        self.state.shell_output.push_str(&output);
    }

    pub(super) fn on_shell_command_finished(&mut self, generation: u64, error: Option<String>) {
        if generation != self.shell_generation {
            return;
        }
        self.shell_stop_tx = None;
        self.state.is_busy = false;
        self.state.is_shell_busy = false;
        self.state.shell_output.push('\n');
        match error {
            Some(error) => {
                self.state
                    .shell_output
                    .push_str(&format!("执行失败: {error}\n"));
                self.state.status_message = "命令执行失败".to_string();
            }
            None => self.state.status_message = "命令执行完成".to_string(),
        }
    }
}
