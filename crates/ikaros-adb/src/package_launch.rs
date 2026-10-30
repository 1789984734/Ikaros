use std::time::Duration;

use ikaros_domain::{DeviceRepository, DomainError};

use crate::{package::valid_package_name, AdbClient};

const SETTINGS_PACKAGE: &str = "com.android.settings";
const SETTINGS_ACTION: &str = "android.settings.SETTINGS";
const LAUNCHER_INTENT: &str = "-a android.intent.action.MAIN -c android.intent.category.LAUNCHER";

impl AdbClient {
    pub(crate) async fn start_package(
        &self,
        serial: &str,
        package: &str,
    ) -> Result<(), DomainError> {
        if !valid_package_name(package) {
            return Err(DomainError::Other("无效的应用包名".into()));
        }
        let component = if package == SETTINGS_PACKAGE {
            self.settings_home_component(serial).await?
        } else {
            self.resolve_component(serial, package, &format!("{LAUNCHER_INTENT} -p {package}"))
                .await?
                .ok_or_else(|| DomainError::Other(format!("应用 {package} 没有可启动的桌面入口")))?
        };
        let intent = if package == SETTINGS_PACKAGE {
            // CLEAR_TOP returns an existing Settings task to its home screen.
            format!("-a {SETTINGS_ACTION} -f 0x14000000")
        } else {
            format!("{LAUNCHER_INTENT} -f 0x10200000")
        };
        // Do not wait for drawing: some OEM activities never report a completed draw.
        let command = format!(
            "am start --user current {intent} -n '{component}'; ikaros_launch_status=$?; echo IKAROS_LAUNCH_EXIT:$ikaros_launch_status"
        );
        let output = self.launch_command(serial, &command).await?;
        check_start_result(&output)
    }

    async fn settings_home_component(&self, serial: &str) -> Result<String, DomainError> {
        let intent = format!("-a {SETTINGS_ACTION} -p {SETTINGS_PACKAGE}");
        if let Some(component) = self
            .resolve_component(serial, SETTINGS_PACKAGE, &intent)
            .await?
        {
            return Ok(component);
        }
        // Some OEM builds expose only a settings subpage to the launcher.
        let intent = format!("-n {SETTINGS_PACKAGE}/.Settings");
        self.resolve_component(serial, SETTINGS_PACKAGE, &intent)
            .await?
            .ok_or_else(|| DomainError::Other("设备未提供可启动的设置首页入口".into()))
    }

    async fn resolve_component(
        &self,
        serial: &str,
        package: &str,
        intent: &str,
    ) -> Result<Option<String>, DomainError> {
        let command = format!("pm resolve-activity --brief --user current {intent}");
        let output = self.launch_command(serial, &command).await?;
        Ok(parse_component(&output, package))
    }

    async fn launch_command(&self, serial: &str, command: &str) -> Result<String, DomainError> {
        tokio::time::timeout(Duration::from_secs(15), self.execute_shell(serial, command))
            .await
            .map_err(|_| DomainError::ShellExecutionFailed("应用启动请求超时".into()))?
    }
}

fn parse_component(output: &str, package: &str) -> Option<String> {
    output.lines().map(str::trim).find_map(|line| {
        let (owner, activity) = line.split_once('/')?;
        (owner == package
            && !activity.is_empty()
            && activity
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'_' | b'$')))
        .then(|| line.to_owned())
    })
}

fn check_start_result(output: &str) -> Result<(), DomainError> {
    let error = output.lines().map(str::trim).find(|line| {
        line.starts_with("Error")
            || line.contains("Exception")
            || line.starts_with("Security exception")
    });
    let accepted = output
        .lines()
        .any(|line| line.trim().starts_with("Starting: Intent"));
    let succeeded = output
        .lines()
        .any(|line| line.trim() == "IKAROS_LAUNCH_EXIT:0");
    if error.is_none() && accepted && succeeded {
        return Ok(());
    }
    let message = output
        .lines()
        .filter(|line| !line.trim().starts_with("IKAROS_LAUNCH_EXIT:"))
        .collect::<Vec<_>>()
        .join("\n");
    let detail = error.unwrap_or(message.trim());
    let detail = if detail.is_empty() {
        "设备未确认应用启动成功".into()
    } else {
        detail.chars().take(400).collect::<String>()
    };
    Err(DomainError::ShellExecutionFailed(detail))
}

#[cfg(test)]
#[path = "package_launch_tests.rs"]
mod tests;
