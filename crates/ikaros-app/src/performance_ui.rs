use crate::{
    mvi::{AppIntent, AppViewState},
    App, AppWindow, PerformanceAppOption, PerformanceChart, PerformanceMetric,
};
use ikaros_domain::{AppInfo, PerformanceSample};
use slint::{Color, ComponentHandle, Model, ModelRc, VecModel};
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
    ui.global::<App>()
        .on_filter_performance_apps(|options, query| {
            let items: Vec<PerformanceAppOption> = options.iter().collect();
            model(filter_app_options(&items, query.as_str()))
        });
}

pub fn apply_choices(ui: &AppWindow, state: &AppViewState) {
    let (options, selected_index, selected_label) = build_app_options(
        &state.performance.packages,
        &state.app_list,
        &state.performance.package,
    );
    let app = ui.global::<App>();
    app.set_performance_selected_index(selected_index as i32);
    app.set_performance_package_label(selected_label.into());
    app.set_performance_app_options(model(options));
}

/// 构建性能监控应用下拉选项列表、当前选中索引及输入框展示文本
fn build_app_options(
    packages: &[String],
    app_list: &[AppInfo],
    current_package: &str,
) -> (Vec<PerformanceAppOption>, usize, String) {
    let mut options = vec![PerformanceAppOption {
        package: "".into(),
        title: "仅查看设备整体".into(),
        subtitle: "".into(),
        is_custom: false,
    }];
    for package in packages {
        let app_name = app_list
            .iter()
            .find(|app| app.package_name == *package)
            .map(|app| app.app_name.as_str())
            .filter(|name| !name.is_empty());
        let (title, subtitle) = match app_name {
            Some(name) => (name.to_owned(), package.clone()),
            None => (package.clone(), String::new()),
        };
        options.push(PerformanceAppOption {
            package: package.into(),
            title: title.into(),
            subtitle: subtitle.into(),
            is_custom: false,
        });
    }
    if !current_package.is_empty()
        && !options
            .iter()
            .any(|opt| opt.package.as_str() == current_package)
    {
        options.push(PerformanceAppOption {
            package: current_package.into(),
            title: "自定义包名".into(),
            subtitle: current_package.into(),
            is_custom: true,
        });
    }
    let selected_index = options
        .iter()
        .position(|opt| opt.package.as_str() == current_package)
        .unwrap_or(0);
    let selected_label = format_selected_label(options.get(selected_index), current_package);
    (options, selected_index, selected_label)
}

fn format_selected_label(option: Option<&PerformanceAppOption>, current_package: &str) -> String {
    if current_package.is_empty() {
        return String::new();
    }
    match option {
        Some(opt) if !opt.is_custom && !opt.subtitle.is_empty() => {
            format!("{} · {}", opt.title, opt.subtitle)
        }
        _ => current_package.to_owned(),
    }
}

/// 按关键字过滤应用列表：支持应用名和包名大小写不敏感筛选，并在需要时提供自定义包名兜底项
fn filter_app_options(options: &[PerformanceAppOption], query: &str) -> Vec<PerformanceAppOption> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return options.to_vec();
    }
    let needle = trimmed.to_lowercase();
    let mut ranked: Vec<(u8, PerformanceAppOption)> = options
        .iter()
        .filter_map(|opt| match_rank(opt, trimmed, &needle).map(|rank| (rank, opt.clone())))
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);

    let mut filtered: Vec<PerformanceAppOption> = ranked.into_iter().map(|(_, opt)| opt).collect();
    let has_exact_package = options.iter().any(|opt| opt.package.as_str() == trimmed);
    if !has_exact_package && is_custom_package_candidate(trimmed) {
        filtered.push(PerformanceAppOption {
            package: trimmed.into(),
            title: "监控自定义包名".into(),
            subtitle: trimmed.into(),
            is_custom: true,
        });
    }
    filtered
}

/// 计算候选项匹配优先级（数值越小越靠前）：精确 > 前缀 > 包名分段前缀 > 子串包含
fn match_rank(option: &PerformanceAppOption, trimmed: &str, needle: &str) -> Option<u8> {
    let title_lower = option.title.to_lowercase();
    let package_lower = option.package.to_lowercase();
    if (!option.package.is_empty() && option.package.eq_ignore_ascii_case(trimmed))
        || title_lower == needle
    {
        return Some(0);
    }
    if title_lower.starts_with(needle) || package_lower.starts_with(needle) {
        return Some(1);
    }
    if package_lower
        .split('.')
        .any(|segment| !segment.is_empty() && segment.starts_with(needle))
    {
        return Some(2);
    }
    if title_lower.contains(needle) || package_lower.contains(needle) {
        return Some(3);
    }
    None
}

/// Android 包名或进程名仅包含无空白的 ASCII 可见字符
fn is_custom_package_candidate(input: &str) -> bool {
    !input.is_empty() && input.is_ascii() && !input.chars().any(char::is_whitespace)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app_info(name: &str, package: &str) -> AppInfo {
        AppInfo {
            app_name: name.into(),
            package_name: package.into(),
            icon_path: None,
            version_name: String::new(),
            version_code: String::new(),
            apk_path: String::new(),
            is_system: false,
            is_enabled: true,
        }
    }

    fn sample_options() -> Vec<PerformanceAppOption> {
        let packages = vec![
            "com.demo.other.mysettings_helper".to_owned(),
            "com.ikko.ab06.settings".to_owned(),
            "com.tencent.mm".to_owned(),
            "com.android.systemui".to_owned(),
        ];
        let apps = vec![
            app_info("SettingsPie", "com.ikko.ab06.settings"),
            app_info("微信", "com.tencent.mm"),
        ];
        build_app_options(&packages, &apps, "").0
    }

    #[test]
    fn build_app_options_formats_selected_label_and_custom_package() {
        let packages = vec![
            "com.ikko.ab06.settings".to_owned(),
            "com.android.systemui".to_owned(),
        ];
        let apps = vec![app_info("SettingsPie", "com.ikko.ab06.settings")];

        let (options, index, label) = build_app_options(&packages, &apps, "");
        assert_eq!(index, 0);
        assert_eq!(label, "");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].title.as_str(), "仅查看设备整体");

        let (_, index, label) = build_app_options(&packages, &apps, "com.ikko.ab06.settings");
        assert_eq!(index, 1);
        assert_eq!(label, "SettingsPie · com.ikko.ab06.settings");

        let (options, index, label) = build_app_options(&packages, &apps, "com.custom.demo");
        assert_eq!(index, 3);
        assert_eq!(label, "com.custom.demo");
        assert!(options[3].is_custom);
        assert_eq!(options[3].package.as_str(), "com.custom.demo");
    }

    #[test]
    fn filter_app_options_prioritizes_prefix_and_segment_matches() {
        let options = sample_options();

        // 空查询保留完整列表
        assert_eq!(filter_app_options(&options, "   ").len(), options.len());

        // "settings"：SettingsPie（标题前缀 rank 1）应排在 mysettings_helper（普通包含 rank 3）之前，末尾附自定义项
        let filtered = filter_app_options(&options, "SETTINGS");
        assert_eq!(filtered[0].package.as_str(), "com.ikko.ab06.settings");
        assert_eq!(
            filtered[1].package.as_str(),
            "com.demo.other.mysettings_helper"
        );
        assert!(filtered[2].is_custom);
        assert_eq!(filtered[2].package.as_str(), "SETTINGS");

        // 精确命中已有包名时不重复追加自定义项
        let exact = filter_app_options(&options, "com.ikko.ab06.settings");
        assert_eq!(exact.len(), 1);
        assert!(!exact[0].is_custom);
        assert_eq!(exact[0].package.as_str(), "com.ikko.ab06.settings");
    }

    #[test]
    fn filter_app_options_handles_chinese_and_custom_packages() {
        let options = sample_options();

        // 中文应用名筛选不生成自定义包名项
        let chinese = filter_app_options(&options, "微信");
        assert_eq!(chinese.len(), 1);
        assert_eq!(chinese[0].package.as_str(), "com.tencent.mm");
        assert!(!chinese[0].is_custom);

        // 未匹配的中文或带空格关键字返回空列表
        assert!(filter_app_options(&options, "不存在的应用").is_empty());
        assert!(filter_app_options(&options, "com foo").is_empty());

        // 未安装的合法包名给出自定义兜底项
        let custom = filter_app_options(&options, "com.hidden.service");
        assert_eq!(custom.len(), 1);
        assert!(custom[0].is_custom);
        assert_eq!(custom[0].package.as_str(), "com.hidden.service");
    }
}
