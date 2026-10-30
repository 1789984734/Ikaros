use super::*;

#[test]
fn parses_android_9_and_15_summaries_and_aggregates_process_memory() {
    let output = "__CPU__\ncpu  10 20 30 100 5 6 7 8 500 500\n__MEM__\nMemTotal: 1024 kB\nMemAvailable: 512 kB\n__BATTERY__\nlevel: 50\nscale: 100\ntemperature: 285\n__PIDS__\n7\n8\n__PROC__\n__PSS__\n** MEMINFO in pid 7 [example] **\nApp Summary\n TOTAL: 100 TOTAL SWAP PSS: 2\n** MEMINFO in pid 8 [example:worker] **\nApp Summary\n TOTAL PSS: 50 TOTAL RSS: 150\n__GFX__\n** Graphics info for pid 7 [example] **\nStats since: 123ns\nTotal frames rendered: 9\nJanky frames: 2 (22%)\nJanky frames (legacy): 6 (66%)\n";
    let reading = parse_reading(output).unwrap();
    assert_eq!(reading.cpu.unwrap().total, 186); // guest ticks must not be counted twice.
    assert_eq!(reading.memory_available_kb, Some(512));
    assert_eq!(reading.battery_temperature_c, Some(28.5));
    assert_eq!(reading.battery_percent, Some(50.0));
    assert_eq!(reading.app_pss_kb, Some(150));
    assert_eq!(reading.process_ids, vec![7, 8]);
    assert_eq!(reading.frames[0].janky, 2);
    assert_eq!(parse_pss("No process found", 1), None);
}

#[test]
fn process_names_with_spaces_parentheses_and_pid_reuse_have_stable_identity() {
    let process =
        parse_process("42 (name (with) spaces) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19")
            .unwrap();
    assert_eq!(process.pid, 42);
    assert_eq!(process.ticks, 23);
    assert_eq!(process.started, 19);
    assert!(parse_process("cat: Permission denied").is_none());
}

#[test]
fn invalid_or_unavailable_metrics_are_not_zero_and_packages_cannot_inject_shell() {
    assert!(sampling_command(Some("com.app; reboot")).is_err());
    assert!(sampling_command(Some("$(id)")).is_err());
    assert!(sampling_command(Some("com.example_app.test")).is_ok());
    assert!(parse_reading("permission denied").is_err());
    let reading = parse_reading("__MEM__\nMemTotal: 200 kB\n__BATTERY__\n").unwrap();
    assert!(reading.cpu.is_none());
    assert!(reading.memory_available_kb.is_none());
    assert!(reading.app_pss_kb.is_none());
}
