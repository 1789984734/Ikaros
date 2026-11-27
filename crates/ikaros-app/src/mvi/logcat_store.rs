use super::{AppIntent, AppStore};
use crate::mvi::log_query::normalize_history_query;
use crate::mvi::log_recording;
use crate::mvi::logcat::stream_log_batches;
use ikaros_domain::{LogEntry, LogLevel};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::{DISPLAY_LOG_LIMIT, LOG_HISTORY_LIMIT};

pub(super) const LOG_FILTER_HISTORY_LIMIT: usize = 10;
const LOG_FILTER_HISTORY_DEBOUNCE: Duration = Duration::from_millis(1500);

impl AppStore {
    pub(super) fn stop_logcat_stream(&mut self) {
        if let Some(stop_tx) = self.logcat_stop_tx.take() {
            let _ = stop_tx.send(true);
        }
        self.logcat_generation = self.logcat_generation.wrapping_add(1);
        self.state.is_logcat_streaming = false;
    }

    /// 重新根据过滤条件计算展示日志
    pub(super) fn recompute_display_logs(&mut self) {
        let level_filter = self.state.log_level_filter;
        let query = crate::mvi::log_query::LogQuery::parse(&self.state.log_search_query);

        let filtered: Vec<LogEntry> = self
            .logs_ring_buffer
            .iter()
            .rev()
            .filter(|e| {
                if let Some(min_level) = level_filter {
                    if e.level < min_level {
                        return false;
                    }
                }
                query.matches(e)
            })
            .take(DISPLAY_LOG_LIMIT)
            .cloned()
            .collect();

        // 恢复时间升序
        let mut list = filtered;
        list.reverse();
        self.state.display_logs = list;
    }

    pub(super) fn on_start_logcat_stream(&mut self) {
        let serial = self.state.current_serial();
        if serial == "未连接" {
            return;
        }

        self.stop_logcat_stream();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        self.logcat_stop_tx = Some(stop_tx);
        self.state.is_logcat_streaming = true;

        tokio::spawn(stream_log_batches(
            Arc::clone(&self.logcat_usecases),
            serial,
            self.logcat_generation,
            self.intent_sender.clone(),
            stop_rx,
            Some(self.log_recorder.clone()),
        ));
    }

    pub(super) fn on_set_log_recording(&mut self, enabled: bool) {
        if self.state.is_log_recording_busy {
            return;
        }
        self.state.is_log_recording_busy = true;
        self.state.log_recording_message = if enabled {
            "正在开启本地记录...".into()
        } else {
            "正在停止写入并清理本地日志...".into()
        };
        let recorder = self.log_recorder.clone();
        let tx = self.intent_sender.clone();
        let previous = self.state.is_log_recording_enabled;
        tokio::spawn(async move {
            if let Err(error) = recorder.set_enabled(enabled).await {
                let _ = tx.send(AppIntent::LogRecordingUpdated {
                    enabled: previous,
                    error: Some(error),
                });
            }
        });
    }

    pub(super) fn on_log_recording_updated(&mut self, enabled: bool, error: Option<String>) {
        self.state.is_log_recording_enabled = enabled;
        self.state.is_log_recording_busy = false;
        self.state.log_recording_has_error = error.is_some();
        self.state.log_recording_message = if let Some(error) = error {
            self.stop_logcat_stream();
            format!("本地记录失败，已暂停抓取：{error}。已有日志保留，可打开文件夹备份。")
        } else if enabled {
            "全量保存 · 关闭后自动清理".into()
        } else {
            "本地记录已关闭 · 已清理保存的日志".into()
        };
    }

    pub(super) fn on_open_log_recording_directory(&mut self) {
        let directory = PathBuf::from(&self.state.log_recording_directory);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            if let Err(error) = log_recording::open_directory(&directory).await {
                let _ = tx.send(AppIntent::SetStatusMessage {
                    message: format!("打开日志文件夹失败：{error}"),
                    is_busy: false,
                });
            }
        });
    }

    pub(super) fn on_log_entries_received(&mut self, generation: u64, entries: Vec<LogEntry>) {
        if generation != self.logcat_generation || !self.state.is_logcat_streaming {
            return;
        }
        for entry in entries {
            if self.logs_ring_buffer.len() == LOG_HISTORY_LIMIT {
                self.logs_ring_buffer.pop_front();
            }
            self.logs_ring_buffer.push_back(entry);
        }
        self.recompute_display_logs();
    }

    pub(super) fn on_logcat_stream_ended(&mut self, generation: u64, error: Option<String>) {
        if generation != self.logcat_generation {
            return;
        }
        self.stop_logcat_stream();
        if let Some(error) = error {
            self.state.status_message = format!("日志读取失败: {error}");
        }
    }

    pub(super) fn on_set_log_level_filter(&mut self, filter: Option<LogLevel>) {
        self.state.log_level_filter = filter;
        self.recompute_display_logs();
    }

    pub(super) fn on_set_log_search_query(&mut self, query: String) {
        self.state.log_search_query = query.clone();
        self.recompute_display_logs();
        self.cancel_log_filter_history_timer();

        if normalize_history_query(&query).is_none() {
            self.log_filter_dirty = false;
            return;
        }
        self.log_filter_dirty = true;
        let generation = self.log_filter_history_generation;
        let tx = self.intent_sender.clone();
        self.log_filter_history_task = Some(tokio::spawn(async move {
            tokio::time::sleep(LOG_FILTER_HISTORY_DEBOUNCE).await;
            let _ = tx.send(AppIntent::CommitLogFilterHistory {
                generation: Some(generation),
                query,
                force: false,
            });
        }));
    }

    pub(super) fn on_commit_log_filter_history(
        &mut self,
        generation: Option<u64>,
        query: String,
        force: bool,
    ) {
        if let Some(expected) = generation {
            if expected != self.log_filter_history_generation
                || self.state.log_search_query != query
            {
                return;
            }
            self.log_filter_history_task = None;
        } else {
            self.cancel_log_filter_history_timer();
            if !force && !self.log_filter_dirty {
                return;
            }
        }
        self.log_filter_dirty = false;
        self.record_log_filter_history(&query);
    }

    pub(super) fn on_select_log_filter_history(&mut self, query: String) {
        self.cancel_log_filter_history_timer();
        self.log_filter_dirty = false;
        self.state.log_search_query = query.clone();
        self.state.log_search_sync_rev = self.state.log_search_sync_rev.wrapping_add(1);
        self.recompute_display_logs();
        self.record_log_filter_history(&query);
    }

    pub(super) fn on_remove_log_filter_history(&mut self, query: String) {
        self.cancel_log_filter_history_timer();
        self.log_filter_dirty = false;
        let before = self.state.log_filter_history.len();
        self.state.log_filter_history.retain(|item| item != &query);
        if self.state.log_filter_history.len() != before {
            self.persist_log_filter_history();
        }
    }

    pub(super) fn on_clear_log_filter_history(&mut self) {
        self.cancel_log_filter_history_timer();
        self.log_filter_dirty = false;
        if !self.state.log_filter_history.is_empty() {
            self.state.log_filter_history.clear();
            self.persist_log_filter_history();
        }
    }

    pub(super) fn flush_pending_log_filter_history(&mut self) {
        self.cancel_log_filter_history_timer();
        if !self.log_filter_dirty {
            return;
        }
        self.log_filter_dirty = false;
        let query = self.state.log_search_query.clone();
        self.record_log_filter_history(&query);
    }

    pub(super) fn on_add_log_filter(&mut self, field: String, value: String) {
        if field.eq_ignore_ascii_case("level") {
            let level = match value.to_ascii_uppercase().as_str() {
                "V" => LogLevel::Verbose,
                "D" => LogLevel::Debug,
                "I" => LogLevel::Info,
                "W" => LogLevel::Warn,
                "E" | "F" => LogLevel::Error,
                _ => return,
            };
            self.on_set_log_level_filter(Some(level));
            return;
        }
        let Some(field) = crate::log_filter::Field::from_key(&field) else {
            return;
        };
        if let Some(query) = crate::mvi::log_query::LogQuery::append_filter(
            &self.state.log_search_query,
            field,
            &value,
        ) {
            self.cancel_log_filter_history_timer();
            self.log_filter_dirty = false;
            self.state.log_search_query = query.clone();
            self.state.log_search_sync_rev = self.state.log_search_sync_rev.wrapping_add(1);
            self.recompute_display_logs();
            self.record_log_filter_history(&query);
        }
    }

    fn cancel_log_filter_history_timer(&mut self) {
        if let Some(task) = self.log_filter_history_task.take() {
            task.abort();
        }
        self.log_filter_history_generation = self.log_filter_history_generation.wrapping_add(1);
    }

    fn record_log_filter_history(&mut self, raw: &str) {
        let Some(normalized) = normalize_history_query(raw) else {
            return;
        };
        if self.state.log_filter_history.first() == Some(&normalized) {
            return;
        }
        self.state
            .log_filter_history
            .retain(|item| item != &normalized);
        self.state.log_filter_history.insert(0, normalized);
        self.state
            .log_filter_history
            .truncate(LOG_FILTER_HISTORY_LIMIT);
        self.persist_log_filter_history();
    }

    fn persist_log_filter_history(&self) {
        let Some(path) = &self.config_path else {
            return;
        };
        crate::app_config::update_at(path, |config| {
            config.log_filter_history = self.state.log_filter_history.clone();
        });
    }

    pub(super) fn on_clear_logs(&mut self) {
        let resume = self.state.is_logcat_streaming;
        self.stop_logcat_stream();
        self.logs_ring_buffer.clear();
        self.state.display_logs.clear();
        let serial = self.state.current_serial();
        let usecases = Arc::clone(&self.logcat_usecases);
        let tx = self.intent_sender.clone();
        let generation = self.logcat_generation;
        tokio::spawn(async move {
            let error = usecases
                .clear_logs(&serial)
                .await
                .err()
                .map(|error| error.to_string());
            let _ = tx.send(AppIntent::LogcatCleared {
                generation,
                resume,
                error,
            });
        });
    }

    pub(super) fn on_logcat_cleared(
        &mut self,
        generation: u64,
        resume: bool,
        error: Option<String>,
    ) {
        if generation != self.logcat_generation {
            return;
        }
        if let Some(error) = error {
            self.state.status_message = format!("清空日志失败: {error}");
        }
        if resume {
            let _ = self.intent_sender.send(AppIntent::StartLogcatStream);
        }
    }

    pub(super) fn on_export_filtered_logs(&mut self) {
        if self.state.display_logs.is_empty() {
            self.state.status_message = "当前筛选结果为空，无需导出".into();
            return;
        }
        let logs = self.state.display_logs.clone();
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let message = match super::device_store::tool_output_path("filtered", "logcat", "log") {
                Some(path) => match export_logs(&logs, &path) {
                    Ok(()) => format!("已导出 {} 条筛选日志: {:?}", logs.len(), path),
                    Err(error) => format!("日志导出失败: {}", error),
                },
                None => "无法确定保存目录".into(),
            };
            let _ = tx.send(AppIntent::SetStatusMessage {
                message,
                is_busy: false,
            });
        });
    }
}

fn export_logs(logs: &[LogEntry], path: &std::path::Path) -> Result<(), std::io::Error> {
    let mut output = String::new();
    for log in logs {
        output.push_str(&format!(
            "{} {} {}: {}: {}\n",
            log.timestamp,
            log.level.as_str(),
            log.tag,
            log.package_name.as_deref().unwrap_or("—"),
            log.message
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, output)
}
