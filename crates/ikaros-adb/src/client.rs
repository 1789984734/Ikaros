use async_trait::async_trait;
use ikaros_domain::{Device, DeviceDetail, DeviceRepository, DeviceStatus, DomainError};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc::UnboundedSender, watch::Receiver};
use tokio::time::timeout;
use tracing::{debug, warn};

/// 将远程路径包裹成可安全进入 shell 的单引号字面量
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// 本地 ADB Server 客户端实现
#[derive(Debug, Clone)]
pub struct AdbClient {
    server_addr: SocketAddr,
}

impl Default for AdbClient {
    fn default() -> Self {
        Self::new("127.0.0.1:5037".parse().unwrap())
    }
}

impl AdbClient {
    pub fn new(server_addr: SocketAddr) -> Self {
        Self { server_addr }
    }

    /// 建立与 ADB Server 的 TCP 连接（如果未启动则自动尝试拉起守护进程）
    pub(crate) async fn connect(&self) -> Result<TcpStream, DomainError> {
        let first_try = timeout(Duration::from_secs(2), TcpStream::connect(self.server_addr)).await;
        if let Ok(Ok(stream)) = first_try {
            return Ok(stream);
        }

        // 尝试自动唤起后台 ADB 守护服务
        let mut cmd = std::process::Command::new("adb");
        cmd.arg("start-server");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW: 避免 Windows 下弹出命令行黑框
        }
        let _ = cmd.output();
        tokio::time::sleep(Duration::from_millis(500)).await;

        match timeout(Duration::from_secs(3), TcpStream::connect(self.server_addr)).await {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(e)) => Err(DomainError::AdbConnectionFailed(format!(
                "连接 ADB 服务 ({}) 失败: {}",
                self.server_addr, e
            ))),
            Err(_) => Err(DomainError::AdbConnectionFailed(
                "连接 ADB 服务超时".to_string(),
            )),
        }
    }

    /// 向 ADB Server 发送带 4 字节长度前缀的请求并读取状态
    pub(crate) async fn write_request(
        stream: &mut TcpStream,
        command: &str,
    ) -> Result<(), DomainError> {
        let request = format!("{:04x}{}", command.len(), command);
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| DomainError::AdbConnectionFailed(format!("发送命令失败: {}", e)))?;
        Self::check_status(stream).await
    }

    /// 校验响应状态（OKAY 或 FAIL）
    async fn check_status(stream: &mut TcpStream) -> Result<(), DomainError> {
        let mut status = [0u8; 4];
        stream.read_exact(&mut status).await.map_err(|e| {
            DomainError::AdbConnectionFailed(format!("读取 ADB 响应状态失败: {}", e))
        })?;

        match &status {
            b"OKAY" => Ok(()),
            b"FAIL" => {
                let err_msg = Self::read_length_prefixed_string(stream).await?;
                Err(DomainError::AdbConnectionFailed(format!(
                    "ADB 返回错误: {}",
                    err_msg
                )))
            }
            other => Err(DomainError::ParseError(format!(
                "未知的 ADB 状态码: {:?}",
                String::from_utf8_lossy(other)
            ))),
        }
    }

    /// 读取 4 字节十六进制长度前缀的内容
    async fn read_length_prefixed_string(stream: &mut TcpStream) -> Result<String, DomainError> {
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| DomainError::ParseError(format!("读取长度前缀失败: {}", e)))?;

        let hex_str = std::str::from_utf8(&len_buf)
            .map_err(|e| DomainError::ParseError(format!("无效的十六进制长度编码: {}", e)))?;
        let len = usize::from_str_radix(hex_str, 16)
            .map_err(|e| DomainError::ParseError(format!("解析十六进制长度失败: {}", e)))?;

        let mut data = vec![0u8; len];
        stream
            .read_exact(&mut data)
            .await
            .map_err(|e| DomainError::ParseError(format!("读取数据负载失败: {}", e)))?;

        String::from_utf8(data)
            .map_err(|e| DomainError::ParseError(format!("UTF-8 解析失败: {}", e)))
    }

    /// 解析 `host:devices-l` 的返回行
    fn parse_device_line(line: &str) -> Option<Device> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut parts = trimmed.split_whitespace();
        let serial = parts.next()?.to_string();
        let status_str = parts.next().unwrap_or("unknown");
        let status = DeviceStatus::from(status_str);

        let mut product = None;
        let mut model = None;
        let mut device_name = None;
        let mut transport_id = None;

        for part in parts {
            if let Some((key, val)) = part.split_once(':') {
                match key {
                    "product" => product = Some(val.to_string()),
                    "model" => model = Some(val.to_string()),
                    "device" => device_name = Some(val.to_string()),
                    "transport_id" => transport_id = Some(val.to_string()),
                    _ => {}
                }
            }
        }

        Some(Device {
            serial,
            status,
            product,
            model,
            device: device_name,
            transport_id,
        })
    }

    /// 解析 `getprop` 全量输出: 每行形如 `[ro.product.brand]: [Google]`
    pub(crate) fn parse_getprop(output: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for line in output.lines() {
            let line = line.trim();
            let Some((key_part, value_part)) = line.split_once("]: [") else {
                continue;
            };
            let key = key_part.trim_start_matches('[').trim().to_string();
            let value = value_part
                .strip_suffix(']')
                .unwrap_or(value_part)
                .to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
        map
    }

    /// 打开设备通道并进入原始服务（shell:/exec:），返回可用数据流
    async fn open_device_service(
        &self,
        serial: &str,
        service: &str,
    ) -> Result<TcpStream, DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, &format!("host:transport:{}", serial)).await?;
        Self::write_request(&mut stream, service).await?;
        Ok(stream)
    }

    /// 在设备上执行命令，实时回传输出文本；收到停止信号时立即断开（远程 shell 进程随之终止）
    pub(crate) async fn stream_shell_raw(
        &self,
        serial: &str,
        command: &str,
        chunk_tx: UnboundedSender<String>,
        mut stop_rx: Receiver<bool>,
    ) -> Result<(), DomainError> {
        let mut stream = self
            .open_device_service(serial, &format!("shell:{}", command))
            .await?;
        let mut buf = [0u8; 8192];
        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        return Ok(());
                    }
                }
                read = stream.read(&mut buf) => {
                    match read {
                        Ok(0) => return Ok(()),
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                            if chunk_tx.send(chunk).is_err() {
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            return Err(DomainError::ShellExecutionFailed(format!(
                                "读取 Shell 输出流失败: {e}"
                            )));
                        }
                    }
                }
            }
        }
    }

    /// 执行设备端命令并把二进制输出流式写入本地文件，返回写入的字节数
    pub(crate) async fn exec_to_file_raw(
        &self,
        serial: &str,
        command: &str,
        local_path: &std::path::Path,
        mut stop_rx: Receiver<bool>,
    ) -> Result<u64, DomainError> {
        use tokio::io::AsyncWriteExt;
        let mut stream = self
            .open_device_service(serial, &format!("exec:{}", command))
            .await?;
        let mut file = tokio::fs::File::create(local_path).await.map_err(|e| {
            DomainError::FileSyncFailed(format!("创建本地文件 {:?} 失败: {e}", local_path))
        })?;
        let mut written = 0_u64;
        let mut buf = [0u8; 16384];
        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        return Ok(written);
                    }
                }
                read = stream.read(&mut buf) => {
                    match read {
                        Ok(0) => {
                            file.flush().await.map_err(|e| {
                                DomainError::FileSyncFailed(format!("写入本地文件失败: {e}"))
                            })?;
                            return Ok(written);
                        }
                        Ok(n) => {
                            file.write_all(&buf[..n]).await.map_err(|e| {
                                DomainError::FileSyncFailed(format!("写入本地文件失败: {e}"))
                            })?;
                            written += n as u64;
                        }
                        Err(e) => {
                            return Err(DomainError::ShellExecutionFailed(format!(
                                "读取命令输出流失败: {e}"
                            )));
                        }
                    }
                }
            }
        }
    }

    /// 连接一台已开启无线调试的远程设备，返回 ADB Server 的描述文本
    pub(crate) async fn connect_remote_raw(&self, address: &str) -> Result<String, DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, &format!("host:connect:{}", address)).await?;
        Self::read_length_prefixed_string(&mut stream).await
    }

    /// 断开一台远程无线设备，返回 ADB Server 的描述文本
    pub(crate) async fn disconnect_remote_raw(&self, address: &str) -> Result<String, DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, &format!("host:disconnect:{}", address)).await?;
        Self::read_length_prefixed_string(&mut stream).await
    }

    /// 让 USB 设备重启 adbd 并进入 TCP 无线模式
    pub(crate) async fn enable_tcpip_raw(
        &self,
        serial: &str,
        port: u16,
    ) -> Result<String, DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, &format!("host:transport:{}", serial)).await?;
        Self::write_request(&mut stream, &format!("tcpip:{}", port)).await?;
        let mut output = Vec::new();
        stream
            .read_to_end(&mut output)
            .await
            .map_err(|e| DomainError::ShellExecutionFailed(format!("读取 tcpip 响应失败: {e}")))?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }

    async fn get_imeis(
        &self,
        serial: &str,
        sdk_api_level: u32,
        props: &HashMap<String, String>,
    ) -> (Option<String>, Option<String>) {
        let (slot1, slot2) = tokio::join!(
            self.get_imei_for_slot(serial, sdk_api_level, 0),
            self.get_imei_for_slot(serial, sdk_api_level, 1),
        );
        let mut imei = slot1.or_else(|| Self::imei_from_properties(props, 0, slot2.as_deref()));
        // 单卡设备的 getDeviceIdForPhone 可能为无效卡槽返回第一卡槽的号码。
        let imei2 = slot2
            .filter(|value| Some(value.as_str()) != imei.as_deref())
            .or_else(|| Self::imei_from_properties(props, 1, imei.as_deref()));
        if imei.is_none() {
            imei = self
                .get_default_imei(serial)
                .await
                .filter(|value| Some(value.as_str()) != imei2.as_deref());
        }
        (imei, imei2)
    }

    async fn get_imei_for_slot(
        &self,
        serial: &str,
        sdk_api_level: u32,
        slot: usize,
    ) -> Option<String> {
        // IPhoneSubInfo.getDeviceIdForPhone 的事务编号和参数随系统版本变化。
        let command = match sdk_api_level {
            22..=23 => format!("service call iphonesubinfo 3 i32 {slot}"),
            24..=29 => format!("service call iphonesubinfo 3 i32 {slot} s16 com.android.shell"),
            // Parcel 字符串长度为 -1 时表示 null，这里对应 callingFeatureId。
            30.. => format!("service call iphonesubinfo 4 i32 {slot} s16 com.android.shell i32 -1"),
            _ => return None,
        };
        let output = self.execute_shell(serial, &command).await.ok()?;
        Self::parse_imei_from_service_call(&output)
    }

    fn imei_from_properties(
        props: &HashMap<String, String>,
        slot: usize,
        other_imei: Option<&str>,
    ) -> Option<String> {
        let keys = [
            "persist.radio.imei",
            "ro.ril.oem.imei",
            "ril.gsm.imei",
            "gsm.baseband.imei",
        ];
        let numbered = keys
            .iter()
            .filter_map(|key| props.get(&format!("{key}{}", slot + 1)).map(String::as_str));
        let ordered = keys
            .iter()
            .filter_map(|key| props.get(*key)?.split(',').nth(slot));

        numbered
            .chain(ordered)
            .map(str::trim)
            .find(|value| {
                (14..=16).contains(&value.len())
                    && value.bytes().all(|byte| byte.is_ascii_digit())
                    && Some(*value) != other_imei
            })
            .map(str::to_owned)
    }

    async fn get_default_imei(&self, serial: &str) -> Option<String> {
        let commands = [
            "service call iphonesubinfo 1 s16 com.android.shell",
            "service call iphonesubinfo 1",
            "service call iphonesubinfo 2 s16 com.android.shell",
            "service call iphonesubinfo 2",
        ];

        for cmd in commands {
            if let Ok(output) = self.execute_shell(serial, cmd).await {
                if let Some(imei) = Self::parse_imei_from_service_call(&output) {
                    return Some(imei);
                }
            }
        }

        None
    }

    /// 从成功返回的 Binder Parcel 中读取 14–16 位 IMEI。
    fn parse_imei_from_service_call(output: &str) -> Option<String> {
        let header = output
            .lines()
            .find_map(|line| line.trim_start().strip_prefix("0x00000000:"))?;
        let mut words = header.split_whitespace();
        if words.next()? != "00000000" {
            return None;
        }
        let length = usize::from_str_radix(words.next()?, 16).ok()?;
        if !(14..=16).contains(&length) {
            return None;
        }
        let digits: String = output
            .lines()
            .filter_map(|line| {
                let (_, text) = line.split_once('\'')?;
                let (text, _) = text.rsplit_once('\'')?;
                Some(text)
            })
            .flat_map(str::chars)
            .filter(char::is_ascii_digit)
            .collect();
        (digits.len() == length).then_some(digits)
    }
}

#[async_trait]
impl DeviceRepository for AdbClient {
    /// 获取当前所有设备
    async fn list_devices(&self) -> Result<Vec<Device>, DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, "host:devices-l").await?;
        let response = Self::read_length_prefixed_string(&mut stream).await?;

        let devices = response
            .lines()
            .filter_map(Self::parse_device_line)
            .collect();
        Ok(devices)
    }

    /// 监听设备热插拔状态
    async fn track_devices(
        &self,
        tx: tokio::sync::mpsc::UnboundedSender<Vec<Device>>,
    ) -> Result<(), DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, "host:track-devices-l").await?;

        loop {
            match Self::read_length_prefixed_string(&mut stream).await {
                Ok(content) => {
                    let devices: Vec<Device> = content
                        .lines()
                        .filter_map(Self::parse_device_line)
                        .collect();
                    if tx.send(devices).is_err() {
                        debug!("监听设备信道已关闭，停止 track_devices 循环");
                        break;
                    }
                }
                Err(e) => {
                    warn!("读取 track-devices 数据流中断: {}", e);
                    break;
                }
            }
        }
        Ok(())
    }

    /// 执行远程 Shell 命令并捕获全部输出
    async fn execute_shell(&self, serial: &str, command: &str) -> Result<String, DomainError> {
        let mut stream = self.connect().await?;
        // 绑定到指定序列号的设备
        Self::write_request(&mut stream, &format!("host:transport:{}", serial)).await?;
        // 发起 Shell 请求
        Self::write_request(&mut stream, &format!("shell:{}", command)).await?;

        // 持续读取流直到 EOF
        let mut output = Vec::new();
        stream.read_to_end(&mut output).await.map_err(|e| {
            DomainError::ShellExecutionFailed(format!("读取 Shell 输出流失败: {}", e))
        })?;

        Ok(String::from_utf8_lossy(&output).to_string())
    }

    /// 读取指定的 Android 系统属性 (getprop)
    async fn get_property(&self, serial: &str, prop_name: &str) -> Result<String, DomainError> {
        let out = self
            .execute_shell(serial, &format!("getprop {}", prop_name))
            .await?;
        Ok(out.trim().to_string())
    }

    /// 查询设备软硬件详情信息
    async fn get_device_detail(&self, serial: &str) -> Result<DeviceDetail, DomainError> {
        // 一次 getprop 全量读取并解析，避免十几次独立 shell 往返
        let props = self
            .execute_shell(serial, "getprop")
            .await
            .unwrap_or_default();
        let prop_map = Self::parse_getprop(&props);
        let prop = |name: &str| prop_map.get(name).cloned().unwrap_or_default();
        let brand = prop("ro.product.brand");
        let model = prop("ro.product.model");
        let android_version = prop("ro.build.version.release");
        let sdk_api_level = prop("ro.build.version.sdk").parse::<u32>().unwrap_or(0);
        let cpu_abi = prop("ro.product.cpu.abi");
        let mut build_number = prop("ro.build.display.id");
        if build_number.is_empty() {
            build_number = prop("ro.build.id");
        }

        // 获取屏幕物理分辨率 (wm size)
        let wm_size = self
            .execute_shell(serial, "wm size")
            .await
            .unwrap_or_default();
        let screen_resolution = wm_size
            .lines()
            .find_map(|line| {
                if let Some(pos) = line.find("Physical size: ") {
                    Some(line[pos + 15..].trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "未知".to_string());

        // 获取屏幕像素密度 (wm density)
        let wm_density = self
            .execute_shell(serial, "wm density")
            .await
            .unwrap_or_default();
        let screen_density = wm_density
            .lines()
            .find_map(|line| {
                if let Some(pos) = line.find("Physical density: ") {
                    line[pos + 18..].trim().parse::<u32>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        // 获取电池电量 (dumpsys battery)
        let battery_dump = self
            .execute_shell(serial, "dumpsys battery")
            .await
            .unwrap_or_default();
        let battery_level = battery_dump.lines().find_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("level: ") {
                trimmed["level: ".len()..].parse::<u8>().ok()
            } else {
                None
            }
        });

        let ((imei, imei2), network) = tokio::join!(
            self.get_imeis(serial, sdk_api_level, &prop_map),
            self.device_network_info(serial),
        );

        Ok(DeviceDetail {
            serial: serial.to_string(),
            brand,
            model,
            android_version,
            sdk_api_level,
            cpu_abi,
            screen_resolution,
            screen_density,
            battery_level,
            imei,
            imei2,
            wifi_ip: network.wifi_ip,
            wifi_mac: network.wifi_mac,
            bluetooth_mac: network.bluetooth_mac,
            build_number,
        })
    }

    /// 设置端口转发 (adb forward)
    async fn forward_port(
        &self,
        serial: &str,
        local_port: u16,
        remote_abstract: &str,
    ) -> Result<(), DomainError> {
        let mut stream = self.connect().await?;
        let cmd = format!(
            "host-serial:{}:forward:tcp:{};{}",
            serial, local_port, remote_abstract
        );
        Self::write_request(&mut stream, &cmd).await
    }

    /// 设置反向端口代理 (adb reverse)
    async fn reverse_port(
        &self,
        serial: &str,
        remote_port: u16,
        local_abstract: &str,
    ) -> Result<(), DomainError> {
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, &format!("host:transport:{}", serial)).await?;
        let cmd = format!("reverse:forward:tcp:{};{}", remote_port, local_abstract);
        Self::write_request(&mut stream, &cmd).await
    }

    async fn stream_shell(
        &self,
        serial: &str,
        command: &str,
        chunk_tx: UnboundedSender<String>,
        stop_rx: Receiver<bool>,
    ) -> Result<(), DomainError> {
        self.stream_shell_raw(serial, command, chunk_tx, stop_rx)
            .await
    }

    async fn exec_to_file(
        &self,
        serial: &str,
        command: &str,
        local_path: &std::path::Path,
        stop_rx: Receiver<bool>,
    ) -> Result<u64, DomainError> {
        self.exec_to_file_raw(serial, command, local_path, stop_rx)
            .await
    }

    async fn connect_remote(&self, address: &str) -> Result<String, DomainError> {
        self.connect_remote_raw(address).await
    }

    async fn disconnect_remote(&self, address: &str) -> Result<String, DomainError> {
        self.disconnect_remote_raw(address).await
    }

    async fn enable_tcpip(&self, serial: &str, port: u16) -> Result<String, DomainError> {
        self.enable_tcpip_raw(serial, port).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imei_service_call_requires_a_successful_string_reply() {
        let output = "+Result: Parcel(
  0x00000000: 00000000 0000000f 00360038 00300030 '........8.6.0.0.'
  0x00000010: 00300030 00300030 00300030 00300030 '0.0.0.0.0.0.0.0.'
  0x00000020: 00300030 00000031                   '0.0.1...        ')";
        assert_eq!(
            AdbClient::parse_imei_from_service_call(output).as_deref(),
            Some("860000000000001")
        );
        let failed_reply = output.replacen("00000000 0000000f", "ffffffff 0000000f", 1);
        assert_eq!(AdbClient::parse_imei_from_service_call(&failed_reply), None);
    }

    #[test]
    fn imei_properties_preserve_slots_and_ignore_duplicate_values() {
        let properties = AdbClient::parse_getprop(
            "[persist.radio.imei]: [, 860000000000002]\n\
             [persist.radio.imei2]: [860000000000001]",
        );
        assert_eq!(AdbClient::imei_from_properties(&properties, 0, None), None);
        assert_eq!(
            AdbClient::imei_from_properties(&properties, 1, Some("860000000000001")).as_deref(),
            Some("860000000000002")
        );

        let properties = AdbClient::parse_getprop(
            "[ro.ril.oem.imei]: [860000000000003]\n\
             [ro.ril.oem.imei1]: [860000000000001]\n\
             [ro.ril.oem.imei2]: [860000000000002]",
        );
        assert_eq!(
            AdbClient::imei_from_properties(&properties, 0, None).as_deref(),
            Some("860000000000001")
        );
        assert_eq!(
            AdbClient::imei_from_properties(&properties, 1, None).as_deref(),
            Some("860000000000002")
        );
    }

    #[test]
    fn parse_getprop_reads_properties_and_resolves_build_number() {
        let sample = "\
[ro.product.brand]: [Google]
[ro.product.model]: [Pixel 8 Pro]
[ro.build.version.release]: [14]
[ro.build.version.sdk]: [34]
[ro.build.display.id]: [UQ1A.240205.004]
[ro.build.id]: [UQ1A.240205.002]
";
        let map = AdbClient::parse_getprop(sample);
        assert_eq!(
            map.get("ro.product.brand").map(|s| s.as_str()),
            Some("Google")
        );
        let mut build_number = map.get("ro.build.display.id").cloned().unwrap_or_default();
        if build_number.is_empty() {
            build_number = map.get("ro.build.id").cloned().unwrap_or_default();
        }
        assert_eq!(build_number, "UQ1A.240205.004");

        // 当 ro.build.display.id 为空时回退到 ro.build.id
        let fallback_sample = "\
[ro.build.display.id]: []
[ro.build.id]: [AP2A.240805.005]
";
        let map_fallback = AdbClient::parse_getprop(fallback_sample);
        let mut build_number_fallback = map_fallback
            .get("ro.build.display.id")
            .cloned()
            .unwrap_or_default();
        if build_number_fallback.is_empty() {
            build_number_fallback = map_fallback.get("ro.build.id").cloned().unwrap_or_default();
        }
        assert_eq!(build_number_fallback, "AP2A.240805.005");
    }
}
