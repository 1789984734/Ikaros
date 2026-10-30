use super::AppStore;
use crate::mvi::{state::PerformanceState, AppIntent};
use ikaros_domain::PerformanceSample;
use ikaros_usecase::{PerformanceAccumulator, PerformanceUseCases};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc::UnboundedSender, time::MissedTickBehavior};

const HISTORY_LIMIT: usize = 120;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

impl AppStore {
    pub(super) fn stop_performance(&mut self) {
        if let Some(task) = self.performance_task.take() {
            task.abort();
        }
        self.performance_generation = self.performance_generation.wrapping_add(1);
        self.state.performance.is_running = false;
    }

    pub(super) fn start_performance(&mut self) {
        if self.state.performance.is_running || self.state.active_tab != 6 {
            return;
        }
        if self.state.current_device().is_none() {
            self.state.performance.message = "请连接并选择一台设备".into();
            return;
        }
        self.state.performance.is_running = true;
        self.state.performance.has_error = false;
        self.state.performance.message = "正在采样，CPU 和帧率将在第二次采样后显示".into();
        let started = *self.performance_started_at.get_or_insert_with(Instant::now);
        self.performance_task = Some(tokio::spawn(sample_loop(
            self.performance_usecases.clone(),
            self.state.current_serial(),
            self.state.performance.package.clone(),
            self.performance_generation,
            started,
            self.intent_sender.clone(),
        )));
    }

    pub(super) fn toggle_performance(&mut self) {
        if self.state.performance.is_running {
            self.stop_performance();
            self.state.performance.message = "已暂停，保留当前数据和趋势".into();
        } else {
            self.start_performance();
        }
    }

    pub(super) fn select_performance_package(&mut self, package: String) {
        let package = package.trim().to_owned();
        if self.state.performance.package == package {
            return;
        }
        self.stop_performance();
        self.state.performance.package = package;
        self.state.performance.history = Arc::default();
        self.performance_started_at = None;
        self.start_performance();
    }

    pub(super) fn reset_performance_device(&mut self) {
        self.stop_performance();
        if let Some(task) = self.performance_packages_task.take() {
            task.abort();
        }
        self.performance_started_at = None;
        self.state.performance = PerformanceState::default();
        if self.state.active_tab == 6 {
            self.load_performance_packages();
            self.start_performance();
        }
    }

    pub(super) fn load_performance_packages(&mut self) {
        if self.state.current_device().is_none() || !self.state.performance.packages.is_empty() {
            return;
        }
        if self
            .performance_packages_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        let usecases = self.performance_usecases.clone();
        let serial = self.state.current_serial();
        let tx = self.intent_sender.clone();
        self.performance_packages_task = Some(tokio::spawn(async move {
            let result = tokio::time::timeout(REQUEST_TIMEOUT, usecases.packages(&serial))
                .await
                .map_err(|_| "读取超时".to_owned())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(AppIntent::PerformancePackagesLoaded { serial, result });
        }));
    }

    pub(super) fn receive_performance_sample(
        &mut self,
        generation: u64,
        sample: PerformanceSample,
    ) {
        if generation != self.performance_generation || !self.state.performance.is_running {
            return;
        }
        let history = Arc::make_mut(&mut self.state.performance.history);
        if history.len() == HISTORY_LIMIT {
            history.pop_front();
        }
        history.push_back(sample);
    }

    pub(super) fn receive_performance_packages(
        &mut self,
        serial: String,
        result: Result<Vec<String>, String>,
    ) {
        if serial != self.state.current_serial() {
            return;
        }
        match result {
            Ok(packages) => self.state.performance.packages = Arc::new(packages),
            Err(error) => {
                self.state.performance.message =
                    format!("应用列表读取失败：{error}，可直接输入包名");
                self.state.performance.has_error = true;
            }
        }
    }

    pub(super) fn handle_performance_failed(&mut self, generation: u64, error: String) {
        if generation == self.performance_generation {
            self.stop_performance();
            self.state.performance.message = format!("采样失败：{error}");
            self.state.performance.has_error = true;
        }
    }

    pub(super) fn on_export_performance_csv(&mut self) {
        if self.state.performance.history.is_empty() {
            self.state.performance.message = "暂无可导出的采样数据".into();
            return;
        }
        let history = self.state.performance.history.clone();
        let package = self.state.performance.package.clone();
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let label = if package.is_empty() {
                "device"
            } else {
                package.as_str()
            };
            let message = match super::device_store::tool_output_path(label, "perf", "csv") {
                Some(path) => match export_csv(&history, &path) {
                    Ok(()) => format!("性能数据已导出: {:?}", path),
                    Err(error) => format!("性能数据导出失败: {}", error),
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

fn export_csv(
    history: &std::sync::Arc<std::collections::VecDeque<PerformanceSample>>,
    path: &std::path::Path,
) -> Result<(), std::io::Error> {
    let mut csv = String::from(
        "elapsed_seconds,cpu_percent,memory_used_kb,memory_total_kb,battery_percent,battery_temperature_c,\
         network_rx_bytes_per_sec,network_tx_bytes_per_sec,app_cpu_percent,app_pss_kb,app_ui_fps,app_jank_percent\n",
    );
    for sample in history.iter() {
        let field = |value: Option<f64>| value.map(|n| format!("{n:.2}")).unwrap_or_default();
        csv.push_str(&format!(
            "{:.2},{},{},{},{},{},{},{},{},{},{},{}\n",
            sample.elapsed_seconds,
            field(sample.cpu_percent),
            sample
                .memory_used_kb
                .map(|n| n.to_string())
                .unwrap_or_default(),
            sample
                .memory_total_kb
                .map(|n| n.to_string())
                .unwrap_or_default(),
            field(sample.battery_percent),
            field(sample.battery_temperature_c),
            field(sample.network_rx_rate_bps.map(|n| n as f64)),
            field(sample.network_tx_rate_bps.map(|n| n as f64)),
            field(sample.app_cpu_percent),
            sample.app_pss_kb.map(|n| n.to_string()).unwrap_or_default(),
            field(sample.app_ui_fps),
            field(sample.app_jank_percent),
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, csv)
}

impl Drop for AppStore {
    fn drop(&mut self) {
        if let Some(task) = self.performance_task.take() {
            task.abort();
        }
        if let Some(task) = self.performance_packages_task.take() {
            task.abort();
        }
    }
}

async fn sample_loop(
    usecases: Arc<PerformanceUseCases>,
    serial: String,
    package: String,
    generation: u64,
    started: Instant,
    tx: UnboundedSender<AppIntent>,
) {
    let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut accumulator = PerformanceAccumulator::default();
    loop {
        interval.tick().await;
        let target = (!package.is_empty()).then_some(package.as_str());
        let result = tokio::time::timeout(REQUEST_TIMEOUT, usecases.read(&serial, target)).await;
        let reading = match result {
            Ok(Ok(reading)) => reading,
            result => {
                let error = match result {
                    Ok(Err(error)) => error.to_string(),
                    _ => "设备响应超时，请检查连接后重试".into(),
                };
                let _ = tx.send(AppIntent::PerformanceFailed { generation, error });
                return;
            }
        };
        let sample = accumulator.sample(started.elapsed().as_secs_f64(), reading);
        if tx
            .send(AppIntent::PerformanceSampled { generation, sample })
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
#[path = "performance_tests.rs"]
mod tests;
