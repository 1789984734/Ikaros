use super::*;
use ikaros_domain::{CpuCounters, FrameCounters, ProcessCounters};

fn reading(total: u64, idle: u64, ticks: u64, frames: u64, janky: u64) -> PerformanceReading {
    PerformanceReading {
        cpu: Some(CpuCounters { total, idle }),
        memory_total_kb: Some(1000),
        memory_available_kb: Some(400),
        process_ids: vec![1],
        processes: vec![ProcessCounters {
            pid: 1,
            started: 20,
            ticks,
        }],
        frames: vec![FrameCounters {
            pid: 1,
            since: 30,
            rendered: frames,
            janky,
        }],
        ..Default::default()
    }
}

#[test]
fn computes_interval_rates_and_preserves_unavailable_baseline() {
    let mut accumulator = PerformanceAccumulator::default();
    let first = accumulator.sample(1.0, reading(100, 40, 10, 40, 2));
    assert!(first.cpu_percent.is_none());
    assert!(first.app_cpu_percent.is_none());
    assert!(first.app_ui_fps.is_none());
    assert_eq!(first.memory_used_kb, Some(600));
    let second = accumulator.sample(3.0, reading(300, 140, 30, 140, 12));
    assert_eq!(second.cpu_percent, Some(50.0));
    assert_eq!(second.app_cpu_percent, Some(10.0));
    assert_eq!(second.app_ui_fps, Some(50.0));
    assert_eq!(second.app_jank_percent, Some(10.0));
    let idle = accumulator.sample(5.0, reading(500, 300, 30, 140, 12));
    assert_eq!(idle.app_ui_fps, Some(0.0));
    assert!(idle.app_jank_percent.is_none());
}

#[test]
fn app_restart_or_counter_reset_rebuilds_baseline_without_spikes() {
    let mut accumulator = PerformanceAccumulator::default();
    accumulator.sample(0.0, reading(100, 40, 10, 40, 2));
    let mut restarted = reading(300, 100, 900, 900, 100);
    restarted.processes[0].started = 21;
    restarted.frames[0].since = 31;
    let sample = accumulator.sample(2.0, restarted);
    assert!(sample.app_cpu_percent.is_none());
    assert!(sample.app_ui_fps.is_none());
    let sample = accumulator.sample(4.0, reading(10, 1, 1, 1, 0));
    assert!(sample.cpu_percent.is_none());
    assert!(sample.app_cpu_percent.is_none());
    assert!(sample.app_ui_fps.is_none());
}

#[test]
fn missing_process_permissions_and_invalid_memory_remain_unavailable() {
    let mut accumulator = PerformanceAccumulator::default();
    accumulator.sample(0.0, reading(100, 40, 10, 40, 2));
    let mut current = reading(300, 100, 30, 100, 5);
    current.process_ids.push(2);
    current.memory_available_kb = Some(2000);
    let sample = accumulator.sample(2.0, current);
    assert!(sample.app_cpu_percent.is_none());
    assert!(sample.memory_used_kb.is_none());
}
