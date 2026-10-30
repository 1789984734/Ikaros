use std::{collections::HashMap, time::Duration};

use ikaros_domain::{DeviceRepository, DomainError, LogEntry};

use crate::AdbClient;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const PROCESS_COMMAND: &str = "ps -A -o PID,UID,USER,NAME; pm list packages -U";

#[derive(Default)]
pub(crate) struct LogcatProcesses {
    processes: HashMap<u32, Process>,
    packages: HashMap<u32, Vec<String>>,
    users: HashMap<String, u32>,
}

struct Process {
    uid: u32,
    user: String,
    name: String,
}

impl AdbClient {
    pub(crate) async fn logcat_processes(
        &self,
        serial: &str,
    ) -> Result<LogcatProcesses, DomainError> {
        tokio::time::timeout(REQUEST_TIMEOUT, self.execute_shell(serial, PROCESS_COMMAND))
            .await
            .map_err(|_| DomainError::ShellExecutionFailed("日志进程信息读取超时".into()))?
            .map(|output| LogcatProcesses::parse(&output))
    }

    pub(crate) async fn refresh_logcat_processes(
        &self,
        serial: &str,
    ) -> Result<LogcatProcesses, DomainError> {
        tokio::time::sleep(REFRESH_INTERVAL).await;
        self.logcat_processes(serial).await
    }
}

impl LogcatProcesses {
    fn parse(output: &str) -> Self {
        let mut result = Self::default();
        for line in output.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            if let [package, uid, ..] = fields.as_slice() {
                if let (Some(package), Some(uid)) =
                    (package.strip_prefix("package:"), uid.strip_prefix("uid:"))
                {
                    if let Ok(uid) = uid.parse() {
                        result
                            .packages
                            .entry(uid)
                            .or_default()
                            .push(package.to_owned());
                    }
                    continue;
                }
            }
            if let [pid, uid, user, name, ..] = fields.as_slice() {
                if let (Ok(pid), Ok(uid)) = (pid.parse(), uid.parse()) {
                    result.users.insert((*user).to_owned(), uid);
                    result.processes.insert(
                        pid,
                        Process {
                            uid,
                            user: (*user).into(),
                            name: (*name).into(),
                        },
                    );
                }
            }
        }
        for packages in result.packages.values_mut() {
            packages.sort_unstable();
            packages.dedup();
        }
        result
    }

    pub(crate) fn annotate(&self, entry: &mut LogEntry, uid_hint: Option<&str>) {
        let hinted_uid = uid_hint.and_then(|hint| {
            hint.parse()
                .ok()
                .or_else(|| self.users.get(hint).copied())
                .or_else(|| app_uid(hint))
        });
        let process = self.processes.get(&entry.pid).filter(|process| {
            uid_hint.is_none_or(|hint| hinted_uid == Some(process.uid) || hint == process.user)
        });
        entry.process_name = process.map(|process| process.name.clone());
        let Some(uid) = hinted_uid.or_else(|| process.map(|process| process.uid)) else {
            return;
        };
        let Some(packages) = self.packages.get(&uid) else {
            return;
        };
        let process_package = process.map(|process| process.name.split(':').next().unwrap_or(""));
        // A shared UID alone cannot tell which application wrote a log.
        entry.package_name = packages
            .iter()
            .find(|package| Some(package.as_str()) == process_package)
            .or_else(|| (packages.len() == 1).then(|| &packages[0]))
            .cloned();
    }
}

fn app_uid(user: &str) -> Option<u32> {
    let (user, app) = user.strip_prefix('u')?.split_once("_a")?;
    user.parse::<u32>()
        .ok()?
        .checked_mul(100_000)?
        .checked_add(10_000)?
        .checked_add(app.parse().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32) -> LogEntry {
        crate::AdbClient::parse_logcat_line(&format!(
            "10-28 12:34:56.000 {pid} {pid} I Tag: message"
        ))
        .unwrap()
    }

    #[test]
    fn resolves_app_subprocesses_and_unique_uids_without_guessing_shared_owners() {
        let processes = LogcatProcesses::parse("PID UID USER NAME\n12 10123 u0_a123 com.example:worker\n13 1000 system system_server\n14 1000 system com.vendor.app\npackage:com.example uid:10123\npackage:android uid:1000\npackage:com.vendor.app uid:1000\n");
        let mut log = entry(12);
        processes.annotate(&mut log, Some("10123"));
        assert_eq!(log.package_name.as_deref(), Some("com.example"));
        assert_eq!(log.process_name.as_deref(), Some("com.example:worker"));
        let mut log = entry(999);
        processes.annotate(&mut log, Some("u0_a123"));
        assert_eq!(log.package_name.as_deref(), Some("com.example"));
        let mut log = entry(13);
        processes.annotate(&mut log, Some("system"));
        assert!(log.package_name.is_none());
        assert_eq!(log.process_name.as_deref(), Some("system_server"));
        let mut log = entry(14);
        processes.annotate(&mut log, Some("system"));
        assert_eq!(log.package_name.as_deref(), Some("com.vendor.app"));
    }

    #[test]
    fn reused_pid_with_a_different_uid_never_inherits_the_current_process_package() {
        let processes = LogcatProcesses::parse("12 10123 u0_a123 com.current\npackage:com.current uid:10123\npackage:com.previous uid:10124\n");
        let mut log = entry(12);
        processes.annotate(&mut log, Some("10124"));
        assert_eq!(log.package_name.as_deref(), Some("com.previous"));
        assert!(log.process_name.is_none());
        let mut unknown = entry(12);
        processes.annotate(&mut unknown, Some("unknown_user"));
        assert!(unknown.package_name.is_none());
        assert!(unknown.process_name.is_none());
    }
}
