use std::{net::IpAddr, time::Duration};

use ikaros_domain::DeviceRepository;

use crate::AdbClient;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(crate) struct DeviceNetworkInfo {
    pub wifi_ip: Option<String>,
    pub wifi_mac: Option<String>,
    pub bluetooth_mac: Option<String>,
}

impl AdbClient {
    pub(crate) async fn device_network_info(&self, serial: &str) -> DeviceNetworkInfo {
        let (interfaces, wifi, bluetooth_mac) = tokio::join!(
            self.network_command(
                serial,
                "getprop wifi.interface; ip -o addr show 2>/dev/null"
            ),
            self.wifi_connection_info(serial),
            self.bluetooth_address(serial),
        );
        DeviceNetworkInfo {
            wifi_ip: wifi.wifi_ip.or_else(|| wifi_interface_ip(&interfaces)),
            wifi_mac: wifi.wifi_mac,
            bluetooth_mac,
        }
    }

    async fn network_command(&self, serial: &str, command: &str) -> String {
        tokio::time::timeout(REQUEST_TIMEOUT, self.execute_shell(serial, command))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    }

    async fn wifi_connection_info(&self, serial: &str) -> DeviceNetworkInfo {
        let status = self
            .network_command(serial, "cmd wifi status 2>/dev/null")
            .await;
        let mut info = parse_wifi_info(&status);
        if info.wifi_mac.is_none() {
            // Older Android versions lack `cmd wifi status`. Keep only current WifiInfo rows.
            let dump = self
                .network_command(
                    serial,
                    "dumpsys wifi 2>/dev/null | sed -n '/^[[:space:]]*mWifiInfo[ :]/p'",
                )
                .await;
            let fallback = parse_wifi_info(&dump);
            info.wifi_mac = fallback.wifi_mac;
            info.wifi_ip = info.wifi_ip.or(fallback.wifi_ip);
        }
        info
    }

    async fn bluetooth_address(&self, serial: &str) -> Option<String> {
        let status = self
            .network_command(
                serial,
                "dumpsys bluetooth_manager 2>/dev/null | sed -n '/^Bluetooth Status/,/^$/p'",
            )
            .await;
        if let Some(address) = parse_bluetooth_address(&status) {
            return Some(address);
        }
        let setting = self
            .network_command(serial, "settings get secure bluetooth_address 2>/dev/null")
            .await;
        mac_address(setting.trim())
    }
}

fn parse_wifi_info(output: &str) -> DeviceNetworkInfo {
    let candidates: Vec<_> = output
        .lines()
        .map(str::trim)
        .filter(|line| {
            (line.starts_with("WifiInfo:")
                || line.starts_with("mWifiInfo ")
                || line.starts_with("mWifiInfo:"))
                && wifi_field(line, "Supplicant state") == Some("COMPLETED")
                && wifi_field(line, "IsPrimary") != Some("0")
        })
        .collect();
    let line = candidates
        .iter()
        .find(|line| wifi_field(line, "IsPrimary") == Some("1"))
        .copied()
        .or_else(|| candidates.first().copied())
        .unwrap_or("");
    DeviceNetworkInfo {
        wifi_ip: wifi_field(line, "IP").and_then(ip_address),
        wifi_mac: wifi_field(line, "MAC").and_then(mac_address),
        bluetooth_mac: None,
    }
}

fn wifi_field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.rsplit(", ").find_map(|field| {
        let (name, value) = field.split_once(':')?;
        (name.trim() == key).then_some(value.trim())
    })
}

fn wifi_interface_ip(output: &str) -> Option<String> {
    let preferred = output
        .lines()
        .next()
        .filter(|line| !line.trim().is_empty())
        .unwrap_or("wlan0")
        .trim();
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            fields.next()?;
            let interface = fields.next()?.split('@').next()?;
            if interface != preferred
                && !interface.starts_with("wlan")
                && !interface.starts_with("wifi")
            {
                return None;
            }
            if !matches!(fields.next()?, "inet" | "inet6") {
                return None;
            }
            let address = ip_address(fields.next()?)?;
            let priority = (interface != preferred, address.contains(':'));
            Some((priority, address))
        })
        .min_by_key(|(priority, _)| *priority)
        .map(|(_, address)| address)
}

fn ip_address(value: &str) -> Option<String> {
    let address: IpAddr = value
        .trim_start_matches('/')
        .split('/')
        .next()?
        .parse()
        .ok()?;
    if address.is_unspecified() || address.is_loopback() || address.is_multicast() {
        return None;
    }
    Some(address.to_string())
}

fn mac_address(value: &str) -> Option<String> {
    let valid = value.len() == 17
        && value.split(':').count() == 6
        && value
            .split(':')
            .all(|part| part.len() == 2 && part.bytes().all(|ch| ch.is_ascii_hexdigit()));
    let address = value.to_ascii_uppercase();
    (valid
        && !matches!(
            address.as_str(),
            "00:00:00:00:00:00" | "FF:FF:FF:FF:FF:FF" | "02:00:00:00:00:00"
        ))
    .then_some(address)
}

fn parse_bluetooth_address(output: &str) -> Option<String> {
    output
        .lines()
        .skip_while(|line| line.trim() != "Bluetooth Status")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .find_map(|line| {
            line.trim()
                .strip_prefix("address:")
                .and_then(|value| mac_address(value.trim()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_mac_belongs_to_the_device_and_prefers_the_primary_connection() {
        let output = "WifiInfo: SSID: test, BSSID: AA:BB:CC:DD:EE:FF, MAC: 52:3e:c4:7a:67:fb, IP: /192.168.1.8, Supplicant state: COMPLETED, IsPrimary: 1, Trusted: true\n\
            mWifiInfo SSID: secondary, BSSID: AA:BB:CC:DD:EE:FF, MAC: 12:34:56:78:9a:bc, IP: /192.168.2.8, Supplicant state: COMPLETED, IsPrimary: 0, Trusted: true";
        let info = parse_wifi_info(output);
        assert_eq!(info.wifi_ip.as_deref(), Some("192.168.1.8"));
        assert_eq!(info.wifi_mac.as_deref(), Some("52:3E:C4:7A:67:FB"));
        assert!(parse_wifi_info("BSSID: AA:BB:CC:DD:EE:FF")
            .wifi_mac
            .is_none());
    }

    #[test]
    fn older_wifi_dumps_work_without_an_ip_and_disconnected_rows_are_ignored() {
        let output = "mWifiInfo SSID: test, BSSID: AA:BB:CC:DD:EE:FF, MAC: 40:45:da:89:85:02, Supplicant state: COMPLETED, RSSI: -45";
        let info = parse_wifi_info(output);
        assert_eq!(info.wifi_mac.as_deref(), Some("40:45:DA:89:85:02"));
        assert!(info.wifi_ip.is_none());
        assert!(
            parse_wifi_info(&output.replace("COMPLETED", "DISCONNECTED"))
                .wifi_mac
                .is_none()
        );
    }

    #[test]
    fn wifi_ip_excludes_mobile_ethernet_and_loopback_addresses() {
        let output = "wlan1\n\
            1: lo inet 127.0.0.1/8 scope host\n\
            2: rmnet0 inet 10.0.0.2/32 scope global\n\
            3: eth0 inet 192.168.2.2/24 scope global\n\
            4: wlan0 inet 192.168.3.2/24 scope global\n\
            5: wlan1 inet6 2001:db8::8/64 scope global\n\
            5: wlan1 inet 192.168.1.8/24 scope global";
        assert_eq!(wifi_interface_ip(output).as_deref(), Some("192.168.1.8"));
        assert_eq!(
            wifi_interface_ip("\n37: wlan0 inet 192.168.1.9/24 scope global").as_deref(),
            Some("192.168.1.9")
        );
        assert!(wifi_interface_ip("\n2: rmnet0 inet 10.0.0.2/32 scope global").is_none());
    }

    #[test]
    fn hidden_or_invalid_addresses_are_not_displayed_as_real_values() {
        for value in [
            "null",
            "",
            "Permission denied",
            "02:00:00:00:00:00",
            "00:00:00:00:00:00",
            "FF:FF:FF:FF:FF:FF",
            "**:**:**:**:12:34",
            "A:B:C:D:E:F",
        ] {
            assert!(mac_address(value).is_none(), "{value}");
        }
        for value in ["null", "0.0.0.0", "/127.0.0.1", "::"] {
            assert!(ip_address(value).is_none(), "{value}");
        }
    }

    #[test]
    fn bluetooth_address_comes_only_from_the_local_adapter_header() {
        let status = "Bluetooth Status\n  enabled: true\n  address: 40:45:da:40:9a:a0\n  name: test\n\nBonded devices:\n  address: AA:BB:CC:DD:EE:FF";
        assert_eq!(
            parse_bluetooth_address(status).as_deref(),
            Some("40:45:DA:40:9A:A0")
        );
        assert!(parse_bluetooth_address("Bonded devices:\n  address: AA:BB:CC:DD:EE:FF").is_none());
        assert!(parse_bluetooth_address(
            "Bluetooth Status\n  address: null\n\nBonded devices:\n  address: AA:BB:CC:DD:EE:FF"
        )
        .is_none());
    }
}
