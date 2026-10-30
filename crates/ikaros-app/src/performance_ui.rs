use crate::{
    mvi::{AppIntent, AppViewState},
    App, AppWindow, PerformanceChart, PerformanceMetric,
};
use ikaros_domain::PerformanceSample;
use slint::ComponentHandle;
use slint::{Color, ModelRc, SharedString, VecModel};
use std::{collections::VecDeque, rc::Rc};
use tokio::sync::mpsc::UnboundedSender;

pub fn bind(ui: &AppWindow, tx: UnboundedSender<AppIntent>) {
    let toggle_tx = tx.clone();
    ui.global::<App>().on_toggle_performance(move || {
        let _ = toggle_tx.send(AppIntent::TogglePerformance);
    });
    ui.global::<App>()
        .on_select_performance_package(move |package| {
            let _ = tx.send(AppIntent::SelectPerformancePackage(package.to_string()));
        });
}

pub fn apply_choices(ui: &AppWindow, state: &AppViewState) {
    let mut packages: Vec<SharedString> = vec!["".into()];
    let mut labels: Vec<SharedString> = vec!["仅查看设备整体".into()];
    for package in state.performance.packages.iter() {
        packages.push(package.into());
        let name = state
            .app_list
            .iter()
            .find(|app| &app.package_name == package)
            .map(|app| app.app_name.as_str())
            .filter(|name| !name.is_empty());
        labels.push(
            name.map(|name| format!("{name} · {package}"))
                .unwrap_or_else(|| package.clone())
                .into(),
        );
    }
    if !state.performance.package.is_empty()
        && !packages
            .iter()
            .any(|package| package.as_str() == state.performance.package)
    {
        packages.push(state.performance.package.clone().into());
        labels.push(format!("自定义 · {}", state.performance.package).into());
    }
    let index = packages
        .iter()
        .position(|p| p.as_str() == state.performance.package)
        .unwrap_or(0);
    ui.global::<App>()
        .set_performance_selected_index(index as i32);
    ui.global::<App>().set_performance_packages(model(packages));
    ui.global::<App>()
        .set_performance_package_labels(model(labels));
}

pub fn apply(ui: &AppWindow, state: &AppViewState) {
    let performance = &state.performance;
    let sample = performance.history.back().cloned().unwrap_or_default();
    ui.global::<App>()
        .set_performance_running(performance.is_running);
    ui.global::<App>()
        .set_performance_has_error(performance.has_error);
    ui.global::<App>()
        .set_performance_can_export(!performance.history.is_empty());
    ui.global::<App>()
        .set_performance_message(performance.message.clone().into());
    ui.global::<App>()
        .set_performance_package(performance.package.clone().into());
    let status = if !performance.is_running || performance.history.is_empty() {
        performance.message.clone()
    } else {
        format!(
            "每 2 秒采样 · 已保留最近 {} / 120 次采样 · 离开本页自动暂停",
            performance.history.len()
        )
    };
    ui.global::<App>()
        .set_performance_sample_status(status.into());
    let app_status = if performance.history.is_empty() {
        "正在等待应用数据".into()
    } else if !sample.process_list_available {
        "无法读取应用进程列表，请检查系统权限".into()
    } else if sample.process_ids.is_empty() {
        "应用未运行；在设备上启动应用后会自动采样".into()
    } else {
        format!(
            "{} 个进程 · PID {}",
            sample.process_ids.len(),
            sample
                .process_ids
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    ui.global::<App>()
        .set_performance_app_status(app_status.into());
    ui.global::<App>()
        .set_performance_device_metrics(model(device_metrics(&sample)));
    ui.global::<App>()
        .set_performance_app_metrics(model(app_metrics(&sample)));
    ui.global::<App>()
        .set_performance_charts(model(charts(&performance.history)));
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}
fn number(value: Option<f64>, unit: &str) -> String {
    value
        .map(|n| format!("{n:.1}{unit}"))
        .unwrap_or_else(|| "—".into())
}
fn metric(title: &str, value: String, detail: String) -> PerformanceMetric {
    PerformanceMetric {
        title: title.into(),
        value: value.into(),
        detail: detail.into(),
    }
}

fn device_metrics(sample: &PerformanceSample) -> Vec<PerformanceMetric> {
    let memory = sample
        .memory_used_kb
        .zip(sample.memory_total_kb)
        .map(|(used, total)| {
            format!(
                "{:.1} / {:.1} GB",
                used as f64 / 1048576.0,
                total as f64 / 1048576.0
            )
        })
        .unwrap_or_else(|| "—".into());
    vec![
        metric(
            "CPU 使用率",
            number(sample.cpu_percent, "%"),
            "整机容量 100%".into(),
        ),
        metric("内存 已用 / 总量", memory, "已用 = 总量 − 可用内存".into()),
        metric(
            "电池温度",
            number(sample.battery_temperature_c, " °C"),
            "设备电池传感器".into(),
        ),
        metric(
            "剩余电量",
            number(sample.battery_percent, "%"),
            "当前电池电量".into(),
        ),
        metric(
            "网络下载速率",
            number(sample.network_rx_rate_bps.map(|bps| bps / 1024.0), " KB/s"),
            "所有非回环网卡收包速率".into(),
        ),
        metric(
            "网络上传速率",
            number(sample.network_tx_rate_bps.map(|bps| bps / 1024.0), " KB/s"),
            "所有非回环网卡发包速率".into(),
        ),
    ]
}

fn app_metrics(sample: &PerformanceSample) -> Vec<PerformanceMetric> {
    vec![
        metric(
            "应用 CPU",
            number(sample.app_cpu_percent, "%"),
            "整机容量 100%".into(),
        ),
        metric(
            "应用内存 PSS",
            number(sample.app_pss_kb.map(|kb| kb as f64 / 1024.0), " MB"),
            "按比例分摊共享内存".into(),
        ),
        metric(
            "UI 绘制 FPS",
            number(sample.app_ui_fps, ""),
            "静止界面可为 0".into(),
        ),
        metric(
            "卡顿帧比例",
            number(sample.app_jank_percent, "%"),
            "本次采样区间 · HWUI".into(),
        ),
    ]
}

fn charts(history: &VecDeque<PerformanceSample>) -> Vec<PerformanceChart> {
    let memory = history
        .iter()
        .filter_map(|s| s.memory_total_kb)
        .max()
        .unwrap_or(1) as f64
        / 1048576.0;
    let pss = history
        .iter()
        .filter_map(|s| s.app_pss_kb)
        .max()
        .unwrap_or(1024) as f64
        / 1024.0
        * 1.2;
    let fps = history
        .iter()
        .filter_map(|s| s.app_ui_fps)
        .fold(60.0, f64::max);
    vec![
        chart(history, "设备 CPU", 100.0, "%", (93, 146, 255), |s| {
            s.cpu_percent
        }),
        chart(
            history,
            "设备内存已用",
            memory,
            " GB",
            (68, 213, 186),
            |s| s.memory_used_kb.map(|n| n as f64 / 1048576.0),
        ),
        chart(history, "应用 CPU", 100.0, "%", (168, 136, 255), |s| {
            s.app_cpu_percent
        }),
        chart(
            history,
            "应用内存 PSS",
            pss,
            " MB",
            (255, 184, 108),
            |s| s.app_pss_kb.map(|n| n as f64 / 1024.0),
        ),
        chart(history, "UI 绘制 FPS", fps, " FPS", (93, 146, 255), |s| {
            s.app_ui_fps
        }),
        chart(
            history,
            "卡顿帧比例",
            100.0,
            "%",
            (244, 127, 143),
            |s| s.app_jank_percent,
        ),
    ]
}

fn chart(
    history: &VecDeque<PerformanceSample>,
    title: &str,
    upper: f64,
    unit: &str,
    color: (u8, u8, u8),
    value: fn(&PerformanceSample) -> Option<f64>,
) -> PerformanceChart {
    let first = history.front().map(|s| s.elapsed_seconds).unwrap_or(0.0);
    let duration = (history.back().map(|s| s.elapsed_seconds).unwrap_or(first) - first).max(2.0);
    let mut commands = String::new();
    let mut connected = false;
    for sample in history {
        if let Some(value) = value(sample).filter(|n| n.is_finite()) {
            let x = (sample.elapsed_seconds - first) / duration * 100.0;
            let y = 98.0 - (value / upper.max(0.001)).clamp(0.0, 1.0) * 96.0;
            commands.push_str(&format!(
                "{} {x:.2} {y:.2} ",
                if connected { "L" } else { "M" }
            ));
            connected = true;
        } else {
            connected = false;
        }
    }
    PerformanceChart {
        title: title.into(),
        upper_label: format!("0 – {upper:.1}{unit}").into(),
        commands: commands.into(),
        accent: Color::from_rgb_u8(color.0, color.1, color.2),
    }
}
