use crate::{logcat_processes::LogcatProcesses, AdbClient};
use async_trait::async_trait;
use ikaros_domain::{DeviceRepository, DomainError, LogEntry, LogLevel, LogcatRepository};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc::Sender, watch::Receiver};

impl AdbClient {
    /// 解析单行 Logcat threadtime 格式日志
    /// 典型格式: "10-27 19:22:30.123  1234  5678 D TagName : This is log message"
    pub fn parse_logcat_line(line: &str) -> Option<LogEntry> {
        parse_record(line).map(|(entry, _)| entry)
    }

    async fn forward_logcat_lines(
        &self,
        serial: &str,
        stream: TcpStream,
        mut processes: LogcatProcesses,
        tx: Sender<LogEntry>,
        mut stop: Receiver<bool>,
    ) -> Result<(), DomainError> {
        let mut lines = BufReader::new(stream).lines();
        let refresh = self.refresh_logcat_processes(serial);
        tokio::pin!(refresh);
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                result = &mut refresh => {
                    if let Ok(snapshot) = result { processes = snapshot; }
                    refresh.set(self.refresh_logcat_processes(serial));
                }
                result = lines.next_line() => {
                    let Ok(Some(line)) = result else { break };
                    if !send_record(&line, &processes, &tx, &mut stop).await { break; }
                }
            }
        }
        Ok(())
    }
}

async fn send_record(
    line: &str,
    processes: &LogcatProcesses,
    tx: &Sender<LogEntry>,
    stop: &mut Receiver<bool>,
) -> bool {
    let Some((mut entry, uid)) = parse_record(line) else {
        return true;
    };
    processes.annotate(&mut entry, uid);
    tokio::select! {
        biased;
        _ = stop.changed() => false,
        result = tx.send(entry) => result.is_ok(),
    }
}

fn next_field<'a>(input: &mut &'a str) -> Option<&'a str> {
    *input = input.trim_start();
    if input.is_empty() {
        return None;
    }
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    let field = &input[..end];
    *input = &input[end..];
    Some(field)
}

fn parse_record(line: &str) -> Option<(LogEntry, Option<&str>)> {
    let mut input = line.trim();
    if input.is_empty() || input.starts_with("---------") {
        return None;
    }
    let timestamp = format!("{} {}", next_field(&mut input)?, next_field(&mut input)?);
    let first = next_field(&mut input)?;
    let second = next_field(&mut input)?;
    let third = next_field(&mut input)?;
    let (pid, tid, level, uid) = if third.len() == 1 && "VDIWEFA".contains(third) {
        (first, second, third, None)
    } else {
        (second, third, next_field(&mut input)?, Some(first))
    };
    if level.len() != 1 || !"VDIWEFA".contains(level) {
        return None;
    }
    let (tag, message) = input
        .trim_start()
        .split_once(':')
        .unwrap_or(("", input.trim_start()));
    Some((
        LogEntry {
            timestamp,
            pid: pid.parse().ok()?,
            tid: tid.parse().ok()?,
            package_name: None,
            process_name: None,
            level: if level == "A" {
                LogLevel::Fatal
            } else {
                LogLevel::from_char(level.chars().next()?)
            },
            tag: tag.trim().into(),
            message: message.trim_start().into(),
        },
        uid,
    ))
}

#[async_trait]
impl LogcatRepository for AdbClient {
    /// 启动异步 Logcat 流监听
    async fn start_logcat_stream(
        &self,
        serial: &str,
        tx: Sender<LogEntry>,
        mut stop_rx: Receiver<bool>,
    ) -> Result<(), DomainError> {
        let processes = tokio::select! {
            biased;
            _ = stop_rx.changed() => return Ok(()),
            result = self.logcat_processes(serial) => result.unwrap_or_default(),
        };
        let mut stream = self.connect().await?;
        Self::write_request(&mut stream, &format!("host:transport:{}", serial)).await?;
        Self::write_request(&mut stream, "shell:logcat -v threadtime -v uid").await?;

        self.forward_logcat_lines(serial, stream, processes, tx, stop_rx)
            .await
    }

    /// 清除设备日志缓存
    async fn clear_logcat(&self, serial: &str) -> Result<(), DomainError> {
        let _ = self.execute_shell(serial, "logcat -c").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, watch};
    use tokio::time::timeout;

    async fn request(socket: &mut tokio::net::TcpStream) -> String {
        let mut command = Vec::new();
        for _ in 0..2 {
            let mut size = [0; 4];
            socket.read_exact(&mut size).await.unwrap();
            let size = usize::from_str_radix(std::str::from_utf8(&size).unwrap(), 16).unwrap();
            command.resize(size, 0);
            socket.read_exact(&mut command).await.unwrap();
            socket.write_all(b"OKAY").await.unwrap();
        }
        String::from_utf8(command).unwrap()
    }

    #[test]
    fn parses_both_threadtime_formats_without_changing_message_colons() {
        for (line, uid) in [
            ("10-28 12:34:56.789 123 456 I Tag : text: with spaces", None),
            (
                "10-28 12:34:56.789 u0_a123 123 456 I Tag : text: with spaces",
                Some("u0_a123"),
            ),
            (
                "10-28 12:34:56.789 10123 123 456 I Tag : text: with spaces",
                Some("10123"),
            ),
        ] {
            let (entry, actual_uid) = parse_record(line).unwrap();
            assert_eq!(actual_uid, uid);
            assert_eq!((entry.pid, entry.tid), (123, 456));
            assert_eq!(entry.tag, "Tag");
            assert_eq!(entry.message, "text: with spaces");
        }
        assert!(parse_record("--------- beginning of main").is_none());
        assert!(parse_record("not a log line").is_none());
    }

    #[tokio::test]
    async fn stopping_logcat_interrupts_a_full_output_channel() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adb = AdbClient::new(listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut metadata, _) = listener.accept().await.unwrap();
            assert!(request(&mut metadata).await.starts_with("shell:ps -A"));
            metadata.write_all(b"PID UID USER NAME\n1234 10123 u0_a123 com.example\npackage:com.example uid:10123\n").await.unwrap();
            drop(metadata);
            let (mut socket, _) = listener.accept().await.unwrap();
            assert_eq!(
                request(&mut socket).await,
                "shell:logcat -v threadtime -v uid"
            );
            let line = "10-27 19:22:30.123  1234  5678 I Test: bounded log\n";
            socket.write_all(line.repeat(3).as_bytes()).await.unwrap();
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
        });
        let (tx, mut rx) = mpsc::channel(1);
        let capacity = tx.clone();
        let (stop, signal) = tokio::sync::watch::channel(false);
        let stream = tokio::spawn(async move { adb.start_logcat_stream("test", tx, signal).await });
        timeout(Duration::from_secs(2), async {
            assert_eq!(
                rx.recv().await.unwrap().package_name.as_deref(),
                Some("com.example")
            );
            while capacity.capacity() != 0 {
                tokio::task::yield_now().await;
            }
            stop.send(true).unwrap();
            stream.await.unwrap().unwrap();
            server.await.unwrap();
        })
        .await
        .expect("A full log channel must not block pause or socket cleanup");
    }

    #[tokio::test]
    async fn pause_cancels_a_stalled_process_metadata_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let adb = AdbClient::new(listener.local_addr().unwrap());
        let (ready, started) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            let _ = ready.send(());
            let mut byte = [0];
            let closed = socket.read(&mut byte).await;
            assert!(closed.map(|count| count == 0).unwrap_or_else(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                )
            }));
        });
        let (tx, _rx) = mpsc::channel(1);
        let (stop, signal) = watch::channel(false);
        let stream = tokio::spawn(async move { adb.start_logcat_stream("test", tx, signal).await });
        timeout(Duration::from_secs(1), async {
            started.await.unwrap();
            stop.send(true).unwrap();
            stream.await.unwrap().unwrap();
            server.await.unwrap();
        })
        .await
        .unwrap();
    }
}
