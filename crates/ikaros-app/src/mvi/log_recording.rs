use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ikaros_domain::LogEntry;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{mpsc, watch};

use super::AppIntent;

const ENABLED_MARKER: &str = ".recording-enabled";
const FILE_PREFIX: &str = "ikaros-logcat-";
const QUEUED_BATCH_LIMIT: usize = 8;

pub(crate) fn default_directory() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = home.map(|home| home.join("Library/Logs"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".local/state")));
    base.unwrap_or_else(std::env::temp_dir)
        .join("Ikaros/logcat")
}

enum RecordingCommand {
    SetEnabled(bool),
    Append {
        epoch: u64,
        serial: String,
        generation: u64,
        entries: Vec<LogEntry>,
    },
}

/// A bounded queue isolates file IO from both the UI and the store dispatcher.
#[derive(Clone)]
pub(super) struct LogRecorder {
    commands: mpsc::Sender<RecordingCommand>,
    active_epoch: watch::Receiver<Option<u64>>,
}

impl LogRecorder {
    pub(super) fn new(directory: PathBuf, intents: mpsc::UnboundedSender<AppIntent>) -> Self {
        let (commands, receiver) = mpsc::channel(QUEUED_BATCH_LIMIT);
        let (active_epoch, signal) = watch::channel(None);
        let worker = RecordingWorker {
            directory,
            intents,
            active_epoch,
            enabled: false,
            epoch: 0,
            file: None,
        };
        tokio::spawn(worker.run(receiver));
        Self {
            commands,
            active_epoch: signal,
        }
    }

    pub(super) async fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.commands
            .send(RecordingCommand::SetEnabled(enabled))
            .await
            .map_err(|_| "本地日志写入任务已停止".to_string())
    }

    pub(super) async fn record_batch(
        &self,
        serial: &str,
        generation: u64,
        entries: &[LogEntry],
    ) -> Result<(), String> {
        let Some(epoch) = *self.active_epoch.borrow() else {
            return Ok(());
        };
        if entries.is_empty() {
            return Ok(());
        }
        self.commands
            .send(RecordingCommand::Append {
                epoch,
                serial: serial.to_string(),
                generation,
                entries: entries.to_vec(),
            })
            .await
            .map_err(|_| "本地日志写入任务已停止".to_string())
    }
}

struct SessionFile {
    serial: String,
    generation: u64,
    writer: BufWriter<File>,
}

struct RecordingWorker {
    directory: PathBuf,
    intents: mpsc::UnboundedSender<AppIntent>,
    active_epoch: watch::Sender<Option<u64>>,
    enabled: bool,
    epoch: u64,
    file: Option<SessionFile>,
}

impl RecordingWorker {
    async fn run(mut self, mut commands: mpsc::Receiver<RecordingCommand>) {
        let result = self.restore().await;
        self.report(result.err());
        while let Some(command) = commands.recv().await {
            match command {
                RecordingCommand::SetEnabled(enabled) => {
                    let result = self.set_enabled(enabled).await;
                    self.report(result.err());
                }
                RecordingCommand::Append {
                    epoch,
                    serial,
                    generation,
                    entries,
                } => {
                    // Ignore batches queued before disabling or starting another recording.
                    if *self.active_epoch.borrow() == Some(epoch) {
                        if let Err(error) = self.append(&serial, generation, &entries).await {
                            self.file = None;
                            self.report(Some(error));
                        }
                    }
                }
            }
        }
        if let Some(mut file) = self.file.take() {
            let _ = file.writer.flush().await;
        }
    }

    async fn restore(&mut self) -> io::Result<()> {
        self.enabled = fs::try_exists(self.directory.join(ENABLED_MARKER)).await?;
        if self.enabled {
            self.epoch += 1;
        } else {
            clear_recordings(&self.directory).await?;
        }
        Ok(())
    }

    async fn set_enabled(&mut self, enabled: bool) -> io::Result<()> {
        if enabled {
            fs::create_dir_all(&self.directory).await?;
            fs::write(self.directory.join(ENABLED_MARKER), b"").await?;
            self.enabled = true;
            self.epoch = self.epoch.wrapping_add(1);
        } else {
            self.active_epoch.send_replace(None);
            // Each batch is flushed; closing precedes removal, including on Windows.
            self.file = None;
            clear_recordings(&self.directory).await?;
            self.enabled = false;
        }
        Ok(())
    }

    fn report(&self, error: Option<io::Error>) {
        let active = if self.enabled && error.is_none() {
            Some(self.epoch)
        } else {
            None
        };
        self.active_epoch.send_replace(active);
        let _ = self.intents.send(AppIntent::LogRecordingUpdated {
            enabled: self.enabled,
            error: error.map(|error| error.to_string()),
        });
    }

    async fn append(
        &mut self,
        serial: &str,
        generation: u64,
        entries: &[LogEntry],
    ) -> io::Result<()> {
        let same_session = self
            .file
            .as_ref()
            .is_some_and(|file| file.serial == serial && file.generation == generation);
        if !same_session {
            if let Some(mut file) = self.file.take() {
                file.writer.flush().await?;
            }
            self.file = Some(open_session_file(&self.directory, serial, generation).await?);
        }
        let mut text = String::new();
        for entry in entries {
            let _ = writeln!(
                text,
                "{} {:>5} {:>5} {} [{}] {}: {}",
                entry.timestamp,
                entry.pid,
                entry.tid,
                entry.level.as_str(),
                entry
                    .package_name
                    .as_deref()
                    .or(entry.process_name.as_deref())
                    .unwrap_or("—"),
                entry.tag,
                entry.message
            );
        }
        let file = self.file.as_mut().expect("session file was opened");
        file.writer.write_all(text.as_bytes()).await?;
        file.writer.flush().await
    }
}

async fn open_session_file(
    directory: &Path,
    serial: &str,
    generation: u64,
) -> io::Result<SessionFile> {
    let safe_serial: String = serial
        .chars()
        .take(64)
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!(
        "{FILE_PREFIX}{safe_serial}-{timestamp}-{}.log",
        std::process::id()
    );
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(name))
        .await?;
    Ok(SessionFile {
        serial: serial.to_string(),
        generation,
        writer: BufWriter::new(file),
    })
}

async fn clear_recordings(directory: &Path) -> io::Result<()> {
    let mut entries = match fs::read_dir(directory).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(FILE_PREFIX)
            && name.ends_with(".log")
            && !entry.file_type().await?.is_dir()
        {
            fs::remove_file(entry.path()).await?;
        }
    }
    match fs::remove_file(directory.join(ENABLED_MARKER)).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(super) async fn open_directory(directory: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let opener = "xdg-open";
    let status = tokio::process::Command::new(opener)
        .arg(directory)
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("无法打开日志文件夹"))
    }
}

#[cfg(test)]
#[path = "log_recording_tests.rs"]
mod tests;
