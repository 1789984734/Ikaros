use async_trait::async_trait;
use ikaros_domain::{
    CpuCounters, DeviceRepository, DomainError, FrameCounters, PerformanceReading,
    PerformanceRepository, ProcessCounters,
};

use crate::AdbClient;

#[async_trait]
impl PerformanceRepository for AdbClient {
    async fn read_performance(
        &self,
        serial: &str,
        package: Option<&str>,
    ) -> Result<PerformanceReading, DomainError> {
        let output = self
            .execute_shell(serial, &sampling_command(package)?)
            .await?;
        parse_reading(&output)
    }

    async fn performance_packages(&self, serial: &str) -> Result<Vec<String>, DomainError> {
        let output = self.execute_shell(serial, "pm list packages").await?;
        let mut packages: Vec<_> = output
            .lines()
            .filter_map(|line| line.trim().strip_prefix("package:").map(str::to_owned))
            .collect();
        packages.sort_unstable();
        packages.dedup();
        if packages.is_empty() {
            return Err(DomainError::ParseError("未能读取应用列表".into()));
        }
        Ok(packages)
    }
}

fn sampling_command(package: Option<&str>) -> Result<String, DomainError> {
    let mut command = String::from(
        "printf '__CPU__\\n'; head -n 1 /proc/stat; \
         printf '__MEM__\\n'; cat /proc/meminfo; \
         printf '__BATTERY__\\n'; dumpsys -t 2 battery; \
         printf '__NET__\\n'; rx=0; tx=0; \
         for i in $(ls /sys/class/net 2>/dev/null); do \
           [ \"$i\" = \"lo\" ] && continue; \
           r=$(cat /sys/class/net/$i/statistics/rx_bytes 2>/dev/null); \
           t=$(cat /sys/class/net/$i/statistics/tx_bytes 2>/dev/null); \
           rx=$((rx+${r:-0})); tx=$((tx+${t:-0})); \
         done; printf 'rx: %s\\ntx: %s\\n' \"$rx\" \"$tx\"; ",
    );
    if let Some(package) = package {
        // Only Android package identifier characters can reach the remote shell.
        if package.is_empty()
            || !package
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_')
        {
            return Err(DomainError::Other("应用包名格式无效".into()));
        }
        command.push_str(&format!(
            "pids=''; processes=$(ps -A -o PID,NAME); if [ $? -eq 0 ]; then \
             pids=$(printf '%s\\n' \"$processes\" | awk -v p='{package}' '$2 == p || index($2, p \":\") == 1 {{print $1}}'); \
             printf '__PIDS__\\n%s\\n' \"$pids\"; fi; \
             printf '__PROC__\\n'; for pid in $pids; do cat /proc/$pid/stat; done; \
             printf '__PSS__\\n'; for pid in $pids; do dumpsys -t 2 meminfo -s $pid; done; \
             printf '__GFX__\\n'; for pid in $pids; do dumpsys -t 2 gfxinfo $pid; done; "
        ));
    }
    Ok(command)
}

fn section<'a>(output: &'a str, marker: &str) -> &'a str {
    output
        .split_once(marker)
        .map(|(_, tail)| tail.split("__").next().unwrap_or(""))
        .unwrap_or("")
}

fn value(text: &str, key: &str) -> Option<f64> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix(key)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

fn parse_reading(output: &str) -> Result<PerformanceReading, DomainError> {
    let memory = section(output, "__MEM__");
    let battery = section(output, "__BATTERY__");
    let mut reading = PerformanceReading {
        cpu: parse_cpu(section(output, "__CPU__")),
        memory_total_kb: value(memory, "MemTotal:").map(|n| n as u64),
        memory_available_kb: value(memory, "MemAvailable:").map(|n| n as u64),
        battery_temperature_c: value(battery, "temperature:").map(|n| n / 10.0),
        process_ids: section(output, "__PIDS__")
            .split_whitespace()
            .filter_map(|n| n.parse().ok())
            .collect(),
        process_list_available: output.contains("__PIDS__"),
        processes: section(output, "__PROC__")
            .lines()
            .filter_map(parse_process)
            .collect(),
        frames: parse_frames(section(output, "__GFX__")),
        network_rx_bytes: value(section(output, "__NET__"), "rx:").map(|n| n as u64),
        network_tx_bytes: value(section(output, "__NET__"), "tx:").map(|n| n as u64),
        ..Default::default()
    };
    reading.battery_percent = value(battery, "level:")
        .zip(value(battery, "scale:"))
        .filter(|(_, scale)| *scale > 0.0)
        .map(|(level, scale)| level / scale * 100.0);
    reading.app_pss_kb = parse_pss(section(output, "__PSS__"), reading.process_ids.len());
    if reading.cpu.is_none()
        && reading.memory_total_kb.is_none()
        && reading.battery_percent.is_none()
    {
        return Err(DomainError::ParseError(
            "设备未返回可用的性能数据，请检查连接或系统权限".into(),
        ));
    }
    Ok(reading)
}

fn parse_cpu(text: &str) -> Option<CpuCounters> {
    let line = text.lines().find(|line| line.starts_with("cpu "))?;
    let ticks: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if ticks.len() < 4 {
        return None;
    }
    Some(CpuCounters {
        total: ticks.iter().sum(),
        idle: ticks[3] + ticks.get(4).copied().unwrap_or(0),
    })
}

fn parse_process(line: &str) -> Option<ProcessCounters> {
    // comm can contain spaces and parentheses; field 3 starts after the last ')'.
    let (head, tail) = line.rsplit_once(')')?;
    let pid = head.split_once('(')?.0.trim().parse().ok()?;
    let fields: Vec<_> = tail.split_whitespace().collect();
    Some(ProcessCounters {
        pid,
        ticks: fields
            .get(11)?
            .parse::<u64>()
            .ok()?
            .checked_add(fields.get(12)?.parse().ok()?)?,
        started: fields.get(19)?.parse().ok()?,
    })
}

fn parse_pss(text: &str, expected: usize) -> Option<u64> {
    let mut totals = Vec::new();
    for block in text.split("** MEMINFO in pid ").skip(1) {
        let summary = block
            .split_once("App Summary")
            .map(|(_, text)| text)
            .unwrap_or(block);
        let total = value(summary, "TOTAL PSS:").or_else(|| value(summary, "TOTAL:"));
        if let Some(total) = total {
            totals.push(total as u64);
        }
    }
    (expected > 0 && totals.len() == expected).then(|| totals.iter().sum())
}

fn parse_frames(text: &str) -> Vec<FrameCounters> {
    text.split("** Graphics info for pid ")
        .skip(1)
        .filter_map(|block| {
            Some(FrameCounters {
                pid: block.split_whitespace().next()?.parse().ok()?,
                since: block
                    .lines()
                    .find_map(|line| line.strip_prefix("Stats since: "))?
                    .strip_suffix("ns")?
                    .parse()
                    .ok()?,
                rendered: value(block, "Total frames rendered:")? as u64,
                janky: value(block, "Janky frames:")? as u64,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "performance_tests.rs"]
mod tests;
