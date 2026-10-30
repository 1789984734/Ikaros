use std::sync::Arc;
use std::time::Duration;
use super::{AppIntent, AppStore};
use tracing::error;

impl AppStore {
    pub(super) fn on_refresh_devices(&mut self) {
        self.state.is_busy = true;
        self.state.status_message = "正在查询设备...".to_string();

        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            match usecases.get_devices().await {
                Ok(devices) => {
                    let _ = tx.send(AppIntent::DevicesUpdated(devices));
                }
                Err(e) => {
                    error!("刷新设备失败: {}", e);
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("查询设备失败: {}", e),
                        is_busy: false,
                    });
                }
            }
        });
    }

    pub(super) fn on_devices_updated(&mut self, devices: Vec<ikaros_domain::Device>) {
        let previous_serial = self.state.current_serial();
        self.state.devices = devices;
        self.state.is_busy = false;
        self.state.selected_device_idx = self.state.devices
            .iter()
            .position(|device| device.serial == previous_serial)
            .unwrap_or(0);

        if self.state.devices.is_empty() {
            self.state.selected_device_detail = None;
            self.state.status_message = "未检测到任何连接的 Android 设备".to_string();
        } else {
            let serial = self.state.devices[self.state.selected_device_idx].serial.clone();
            self.state.status_message = format!("已连接 {} 台设备", self.state.devices.len());
            let _ = self.intent_sender.send(AppIntent::LoadDeviceDetail(serial));
        }
        if previous_serial != self.state.current_serial() {
            self.state.app_list.clear();
            self.reset_performance_device();
        }
    }

    pub(super) fn on_select_device(&mut self, index: usize) {
        if index < self.state.devices.len() {
            let changed_device = index != self.state.selected_device_idx;
            let was_streaming = self.state.is_logcat_streaming;
            self.stop_logcat_stream();
            if changed_device {
                self.state.app_list.clear();
                self.logs_ring_buffer.clear();
                self.state.display_logs.clear();
            }
            self.state.selected_device_idx = index;
            if changed_device { self.reset_performance_device(); }
            let serial = self.state.devices[index].serial.clone();
            let _ = self.intent_sender.send(AppIntent::LoadDeviceDetail(serial));
            let _ = self.intent_sender.send(AppIntent::BrowseDirectory("/sdcard".to_string()));
            let _ = self.intent_sender.send(AppIntent::LoadPackages(self.state.app_filter));
            if was_streaming {
                let _ = self.intent_sender.send(AppIntent::StartLogcatStream);
            }
        }
    }

    pub(super) fn on_load_device_detail(&mut self, serial: String) {
        self.state.is_busy = true;
        self.state.status_message = format!("正在读取设备 [{}] 详情...", serial);

        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            match usecases.get_device_detail(&serial).await {
                Ok(detail) => {
                    let _ = tx.send(AppIntent::DeviceDetailLoaded(detail));
                }
                Err(e) => {
                    error!("读取设备详情失败: {}", e);
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("读取详情失败: {}", e),
                        is_busy: false,
                    });
                }
            }
        });
    }

    pub(super) fn on_device_detail_loaded(&mut self, detail: ikaros_domain::DeviceDetail) {
        if detail.serial != self.state.current_serial() { return; }
        let msg = format!("已连接: {} ({})", detail.model, detail.serial);
        self.state.status_message = msg;
        self.state.is_busy = false;

        self.state.shell_output.push_str(&format!(
            "=== 设备 [{}] 详情已就绪 ===\n品牌: {}\n型号: {}\n系统版本: Android {} (API {})\n架构: {}\n分辨率: {}\n电量: {:?}%\n\n",
            detail.serial, detail.brand, detail.model, detail.android_version, detail.sdk_api_level, detail.cpu_abi, detail.screen_resolution, detail.battery_level
        ));

        self.state.selected_device_detail = Some(detail);

        let _ = self.intent_sender.send(AppIntent::BrowseDirectory("/sdcard".to_string()));
        let _ = self.intent_sender.send(AppIntent::LoadPackages(self.state.app_filter));
    }

    pub(super) fn on_switch_tab(&mut self, tab: i32) {
        self.state.active_tab = tab;
        if tab == 6 {
            self.load_performance_packages();
            self.start_performance();
        } else {
            self.stop_performance();
        }
        if tab == 2 && self.state.file_list.is_empty() {
            let path = self.state.current_dir.clone();
            let _ = self.intent_sender.send(AppIntent::BrowseDirectory(path));
        } else if tab == 3 && self.state.app_list.is_empty() {
            let filter = self.state.app_filter;
            let _ = self.intent_sender.send(AppIntent::LoadPackages(filter));
        } else if tab == 4 && !self.state.is_logcat_streaming {
            let _ = self.intent_sender.send(AppIntent::StartLogcatStream);
        }
    }

    // ========== 无线调试 ==========
    pub(super) fn on_wireless_connect(&mut self, address: String) {
        let address = address.trim().to_string();
        if address.is_empty() {
            self.state.status_message = "请输入无线调试地址 (ip:port)".into();
            return;
        }
        if !address.contains(':') {
            self.state.status_message = format!("地址需包含端口，例如 {}:5555", address);
            return;
        }
        self.state.is_busy = true;
        self.state.status_message = format!("正在连接无线设备 {}...", address);
        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let message = match usecases.connect_remote(&address).await {
                Ok(text) => format!("无线连接成功: {}", text.trim()),
                Err(e) => format!("无线连接失败: {}", e),
            };
            let _ = tx.send(AppIntent::SetStatusMessage { message, is_busy: false });
            let _ = tx.send(AppIntent::RefreshDevices);
        });
    }

    pub(super) fn on_wireless_disconnect(&mut self, address: String) {
        let address = address.trim().to_string();
        if address.is_empty() {
            return;
        }
        self.state.is_busy = true;
        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let message = match usecases.disconnect_remote(&address).await {
                Ok(text) => format!("已断开: {}", text.trim()),
                Err(e) => format!("断开失败: {}", e),
            };
            let _ = tx.send(AppIntent::SetStatusMessage { message, is_busy: false });
            let _ = tx.send(AppIntent::RefreshDevices);
        });
    }

    pub(super) fn on_enable_wireless_mode(&mut self) {
        let Some(device) = self.state.current_device().cloned() else {
            self.state.status_message = "请先连接并选择一台 USB 设备".into();
            return;
        };
        let wifi_ip = self.state.selected_device_detail.as_ref()
            .filter(|detail| detail.serial == device.serial)
            .and_then(|detail| detail.wifi_ip.clone());
        let Some(wifi_ip) = wifi_ip else {
            self.state.status_message = "无法获取设备 Wi-Fi IP，请确认设备与电脑在同一网络".into();
            return;
        };

        self.state.is_busy = true;
        self.state.status_message = "正在切换设备到 TCP 无线模式...".into();
        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        let serial = device.serial.clone();
        tokio::spawn(async move {
            let result = usecases.enable_wireless(&serial, 5555).await;
            match result {
                Ok(_) => {
                    // adbd 重启需要几秒才能接受连接
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    let address = format!("{}:5555", wifi_ip);
                    let message = match usecases.connect_remote(&address).await {
                        Ok(text) => format!("无线模式已开启: {}", text.trim()),
                        Err(e) => format!("已切换无线模式，但回连 {} 失败: {}（可在手机上确认无线调试端口后手动连接）", address, e),
                    };
                    let _ = tx.send(AppIntent::SetStatusMessage { message, is_busy: false });
                    let _ = tx.send(AppIntent::RefreshDevices);
                }
                Err(e) => {
                    let _ = tx.send(AppIntent::SetStatusMessage {
                        message: format!("切换无线模式失败: {}", e),
                        is_busy: false,
                    });
                }
            }
        });
    }

    // ========== 快捷工具 ==========
    pub(super) fn on_capture_screenshot(&mut self) {
        let serial = self.state.current_serial();
        if serial == "未连接" { return; }
        let Some(path) = tool_output_path(&serial, "screenshot", "png") else {
            self.state.status_message = "无法确定保存目录".into();
            return;
        };
        self.state.is_busy = true;
        self.state.status_message = "正在截取设备屏幕...".into();
        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let (_, stop_rx) = tokio::sync::watch::channel(false);
            let message = match usecases
                .exec_to_file(&serial, "screencap -p", &path, stop_rx)
                .await
            {
                Ok(0) => "截图失败: 设备未返回数据".to_string(),
                Ok(bytes) => format!("截图已保存: {:?} ({:.1} KB)", path, bytes as f64 / 1024.0),
                Err(e) => format!("截图失败: {}", e),
            };
            let _ = tx.send(AppIntent::SetStatusMessage { message, is_busy: false });
        });
    }

    pub(super) fn on_capture_screen_recording(&mut self, seconds: u32) {
        let serial = self.state.current_serial();
        if serial == "未连接" { return; }
        let seconds = seconds.clamp(3, 180);
        let device_path = format!("/sdcard/ikaros_record_{}.mp4", timestamp());
        let Some(local_path) = tool_output_path(&serial, "record", "mp4") else {
            self.state.status_message = "无法确定保存目录".into();
            return;
        };
        self.state.is_busy = true;
        self.state.status_message = format!("正在录制设备屏幕 {} 秒，期间请勿断开设备...", seconds);
        let usecases = Arc::clone(&self.device_usecases);
        let file_usecases = Arc::clone(&self.file_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let message = match usecases
                .execute_shell(&serial, &format!("screenrecord --time-limit {} {}", seconds, shell_quote(&device_path)))
                .await
            {
                Ok(_) => {
                    std::fs::create_dir_all(local_path.parent().unwrap_or(std::path::Path::new("/tmp"))).ok();
                    match file_usecases.pull_file(&serial, &device_path, &local_path, None).await {
                        Ok(()) => format!("录屏已保存: {:?}", local_path),
                        Err(e) => format!("录制完成，但拉取视频失败: {}（设备端路径 {}）", e, device_path),
                    }
                }
                Err(e) => format!("录屏失败: {}", e),
            };
            let _ = usecases.execute_shell(&serial, &format!("rm -f {}", shell_quote(&device_path))).await.ok();
            let _ = tx.send(AppIntent::SetStatusMessage { message, is_busy: false });
        });
    }

    pub(super) fn on_capture_bugreport(&mut self) {
        let serial = self.state.current_serial();
        if serial == "未连接" { return; }
        let Some(path) = tool_output_path(&serial, "bugreport", "zip") else {
            self.state.status_message = "无法确定保存目录".into();
            return;
        };
        self.state.is_busy = true;
        self.state.status_message = "正在生成 Bug 报告，通常需要 1~2 分钟，请保持设备连接...".into();
        let usecases = Arc::clone(&self.device_usecases);
        let tx = self.intent_sender.clone();
        tokio::spawn(async move {
            let (_, stop_rx) = tokio::sync::watch::channel(false);
            let message = match usecases
                .exec_to_file(&serial, "bugreportz -s", &path, stop_rx)
                .await
            {
                Ok(bytes) if bytes > 0 => format!("Bug 报告已保存: {:?} ({:.1} MB)", path, bytes as f64 / 1048576.0),
                Ok(_) => "Bug 报告生成失败: 设备未返回数据（可能系统不支持 bugreportz -s）".to_string(),
                Err(e) => format!("Bug 报告生成失败: {}", e),
            };
            let _ = tx.send(AppIntent::SetStatusMessage { message, is_busy: false });
        });
    }
}

/// 快捷工具产物的本地保存路径: ~/Downloads/Ikaros/
pub(super) fn tool_output_path(serial: &str, kind: &str, ext: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut path = std::path::PathBuf::from(home);
    path.push("Downloads/Ikaros");
    std::fs::create_dir_all(&path).ok()?;
    let safe_serial: String = serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' { c } else { '_' })
        .collect();
    path.push(format!("ikaros-{}-{}-{}.{}", kind, safe_serial, timestamp(), ext));
    Some(path)
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 以本地时间近似展示（不做完整时区换算，仅用于文件名唯一性）
    format!("{}", now)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}
