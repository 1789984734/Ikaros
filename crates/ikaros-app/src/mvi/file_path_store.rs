use super::{AppIntent, AppStore};
use std::sync::Arc;
use std::time::Duration;

impl AppStore {
    pub(super) fn on_complete_file_path(&mut self, query: String) {
        self.clear_file_path_completions();
        let serial = self.state.current_serial();
        if serial == "未连接" || query.trim().is_empty() {
            return;
        }
        self.state.file_path_completion_query = query.clone();
        let generation = self.file_completion_generation;
        let current = self.state.current_dir.clone();
        let usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        self.file_completion_task = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(180)).await;
            let choices = tokio::time::timeout(
                Duration::from_secs(3),
                usecases.complete_directory_path(&serial, &current, &query),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            let _ = tx.send(AppIntent::FilePathCompletionsLoaded {
                serial,
                generation,
                choices,
            });
        }));
    }

    pub(super) fn on_file_path_completions_loaded(
        &mut self,
        serial: String,
        generation: u64,
        choices: Vec<String>,
    ) {
        if serial != self.state.current_serial() || generation != self.file_completion_generation {
            return;
        }
        self.file_completion_task = None;
        self.state.file_path_suggestions = choices;
    }

    pub(super) fn clear_file_path_completions(&mut self) {
        if let Some(task) = self.file_completion_task.take() {
            task.abort();
        }
        self.file_completion_generation = self.file_completion_generation.wrapping_add(1);
        self.state.file_path_completion_query.clear();
        self.state.file_path_suggestions.clear();
    }

    pub(super) fn reset_file_navigation(&mut self) {
        if let Some(task) = self.file_browse_task.take() {
            task.abort();
        }
        self.file_browse_generation = self.file_browse_generation.wrapping_add(1);
        self.clear_file_path_completions();
        self.state.current_dir = "/sdcard".into();
        self.state.file_list.clear();
        if self.file_transfer_cancel.is_none() {
            self.state.file_transfer_message.clear();
            self.state.file_transfer_percent = -1.0;
        }
    }
}
