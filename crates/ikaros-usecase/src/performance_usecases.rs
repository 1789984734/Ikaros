use ikaros_domain::{DomainError, PerformanceReading, PerformanceRepository, PerformanceSample};
use std::sync::Arc;

pub struct PerformanceUseCases {
    repo: Arc<dyn PerformanceRepository>,
}

impl PerformanceUseCases {
    pub fn new(repo: Arc<dyn PerformanceRepository>) -> Self {
        Self { repo }
    }

    pub async fn read(
        &self,
        serial: &str,
        package: Option<&str>,
    ) -> Result<PerformanceReading, DomainError> {
        self.repo.read_performance(serial, package).await
    }

    pub async fn packages(&self, serial: &str) -> Result<Vec<String>, DomainError> {
        self.repo.performance_packages(serial).await
    }
}

/// A new accumulator is created for every device/target/resume session.
#[derive(Default)]
pub struct PerformanceAccumulator {
    previous: Option<(f64, PerformanceReading)>,
}

impl PerformanceAccumulator {
    pub fn sample(&mut self, seconds: f64, reading: PerformanceReading) -> PerformanceSample {
        let mut sample = PerformanceSample {
            elapsed_seconds: seconds,
            memory_total_kb: reading.memory_total_kb,
            memory_used_kb: reading
                .memory_total_kb
                .zip(reading.memory_available_kb)
                .and_then(|(total, available)| total.checked_sub(available)),
            battery_percent: reading.battery_percent,
            battery_temperature_c: reading.battery_temperature_c,
            process_ids: reading.process_ids.clone(),
            process_list_available: reading.process_list_available,
            app_pss_kb: reading.app_pss_kb,
            ..Default::default()
        };
        if let Some((previous_seconds, previous)) = &self.previous {
            sample.cpu_percent = cpu_percent(previous, &reading);
            sample.app_cpu_percent = app_cpu_percent(previous, &reading);
            let (fps, jank) = frame_rates(previous, &reading, seconds - previous_seconds);
            sample.app_ui_fps = fps;
            sample.app_jank_percent = jank;
            let elapsed = seconds - previous_seconds;
            sample.network_rx_rate_bps =
                network_rate(previous.network_rx_bytes, reading.network_rx_bytes, elapsed);
            sample.network_tx_rate_bps =
                network_rate(previous.network_tx_bytes, reading.network_tx_bytes, elapsed);
        }
        self.previous = Some((seconds, reading));
        sample
    }
}

fn network_rate(previous: Option<u64>, current: Option<u64>, seconds: f64) -> Option<f64> {
    let delta = current?.checked_sub(previous?)?;
    let rate = delta as f64 / seconds.max(0.001);
    (rate.is_finite() && rate >= 0.0).then_some(rate)
}

fn cpu_percent(previous: &PerformanceReading, current: &PerformanceReading) -> Option<f64> {
    let old = previous.cpu.as_ref()?;
    let new = current.cpu.as_ref()?;
    let total = new.total.checked_sub(old.total).filter(|n| *n > 0)?;
    let idle = new.idle.checked_sub(old.idle)?;
    Some(100.0 * total.checked_sub(idle)? as f64 / total as f64)
}

fn app_cpu_percent(previous: &PerformanceReading, current: &PerformanceReading) -> Option<f64> {
    let total = current
        .cpu
        .as_ref()?
        .total
        .checked_sub(previous.cpu.as_ref()?.total)?;
    if total == 0
        || current.process_ids.is_empty()
        || current.processes.len() != current.process_ids.len()
    {
        return None;
    }
    let ticks = current.processes.iter().try_fold(0_u64, |sum, process| {
        let old = previous
            .processes
            .iter()
            .find(|p| p.pid == process.pid && p.started == process.started)?;
        sum.checked_add(process.ticks.checked_sub(old.ticks)?)
    })?;
    Some((100.0 * ticks as f64 / total as f64).clamp(0.0, 100.0))
}

fn frame_rates(
    previous: &PerformanceReading,
    current: &PerformanceReading,
    seconds: f64,
) -> (Option<f64>, Option<f64>) {
    if seconds <= 0.0 || current.frames.is_empty() {
        return (None, None);
    }
    let delta = current
        .frames
        .iter()
        .try_fold((0_u64, 0_u64), |(frames, janky), frame| {
            let old = previous
                .frames
                .iter()
                .find(|p| p.pid == frame.pid && p.since == frame.since)?;
            Some((
                frames.checked_add(frame.rendered.checked_sub(old.rendered)?)?,
                janky.checked_add(frame.janky.checked_sub(old.janky)?)?,
            ))
        });
    match delta {
        Some((frames, janky)) => (
            Some(frames as f64 / seconds),
            (frames > 0).then(|| 100.0 * janky as f64 / frames as f64),
        ),
        None => (None, None),
    }
}

#[cfg(test)]
#[path = "performance_tests.rs"]
mod tests;
