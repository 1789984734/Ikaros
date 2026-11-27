//! Detect the device-side HEVC failure without treating a static display as a stall.

use ikaros_adb::AdbClient;
use ikaros_domain::LogLevel;
use ikaros_usecase::DeviceUseCases;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::debug;

// Read at most one existing entry so a restarted encoder cannot inherit the old failure.
const LOGCAT_COMMAND: &str = "logcat -b main -v threadtime -T 1 -s VSP:E SPRDHEVCEncoder:I";
const FAILED_FRAMES: u8 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecoveryStep {
    #[default]
    Initial,
    RetryH265,
    FallbackH264,
}

impl RecoveryStep {
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Initial => Some(Self::RetryH265),
            Self::RetryH265 => Some(Self::FallbackH264),
            Self::FallbackH264 => None,
        }
    }
}

#[derive(Default)]
struct FailedFrames {
    hardware_error: Option<Instant>,
    first_failure: Option<Instant>,
    count: u8,
}

impl FailedFrames {
    fn record_output(&mut self, bytes: usize, now: Instant) -> bool {
        let hardware_error = self.hardware_error.take();
        if bytes != 0 || !hardware_error.is_some_and(|time| now - time <= FAILURE_WINDOW) {
            *self = Self::default();
            return false;
        }
        if self
            .first_failure
            .is_none_or(|first| now - first > FAILURE_WINDOW)
        {
            self.first_failure = Some(now);
            self.count = 0;
        }
        self.count += 1;
        self.count >= FAILED_FRAMES
    }
}

#[derive(Default)]
struct HevcStallDetector {
    partial_line: String,
    threads: HashMap<(u32, u32), FailedFrames>,
}

impl HevcStallDetector {
    fn observe_chunk(&mut self, chunk: &str, now: Instant) -> bool {
        self.partial_line.push_str(chunk);
        let Some(last_newline) = self.partial_line.rfind('\n') else {
            return false;
        };
        let remaining = self.partial_line.split_off(last_newline + 1);
        let complete_lines = std::mem::replace(&mut self.partial_line, remaining);
        complete_lines.lines().any(|line| self.observe(line, now))
    }

    fn observe(&mut self, line: &str, now: Instant) -> bool {
        let Some(entry) = AdbClient::parse_logcat_line(line) else {
            return false;
        };
        let thread = (entry.pid, entry.tid);
        if entry.tag == "VSP"
            && entry.level >= LogLevel::Error
            && entry.message.starts_with("H265Enc_WriteSlice,")
            && entry.message.contains("BSM or IOMMU error")
        {
            self.threads.entry(thread).or_default().hardware_error = Some(now);
            return false;
        }
        if entry.tag != "SPRDHEVCEncoder" || !entry.message.starts_with("H265EncStrmEncode[") {
            return false;
        }
        let Some(bytes) = output_size(&entry.message) else {
            return false;
        };
        self.threads
            .get_mut(&thread)
            .is_some_and(|failures| failures.record_output(bytes, now))
    }
}

fn output_size(message: &str) -> Option<usize> {
    let (_, output) = message.split_once("out {")?;
    let (fields, _) = output.split_once('}')?;
    let (_, bytes) = fields.rsplit_once(',')?;
    bytes.trim().parse().ok()
}

pub async fn wait_for_hevc_stall(devices: &DeviceUseCases, serial: &str) {
    let (_lifetime, stop) = watch::channel(false);
    let (chunks, mut output) = mpsc::unbounded_channel();
    let stream = devices.stream_shell(serial, LOGCAT_COMMAND, chunks, stop);
    tokio::pin!(stream);
    let mut detector = HevcStallDetector::default();
    loop {
        let chunk = tokio::select! {
            biased;
            Some(chunk) = output.recv() => chunk,
            result = &mut stream => {
                debug!(%serial, ?result, "H.265 日志监听已结束");
                break;
            }
        };
        if detector.observe_chunk(&chunk, Instant::now()) {
            return;
        }
    }
    // Stream exit or failure alone is never evidence of an encoding stall.
    std::future::pending().await
}

#[cfg(test)]
mod tests {
    use super::*;

    const HARDWARE_ERROR: &str =
        "11-26 10:57:02.085 401 6097 E VSP : H265Enc_WriteSlice, 564, BSM or IOMMU error\n";
    const EMPTY_FRAME: &str = "11-26 10:57:02.112 401 6097 I SPRDHEVCEncoder: H265EncStrmEncode[300] 31ms, in {0xe6898000-0x10229000, 448x592, 5}, out {0xead39000-0, 0}, wh{0, 0}\n";

    #[test]
    fn confirms_repeated_hardware_errors_even_with_split_log_lines() {
        let mut detector = HevcStallDetector::default();
        let now = Instant::now();
        for index in 1..=FAILED_FRAMES {
            let (first, rest) = HARDWARE_ERROR.split_at(34);
            assert!(!detector.observe_chunk(first, now));
            assert!(!detector.observe_chunk(rest, now));
            assert_eq!(
                detector.observe_chunk(EMPTY_FRAME, now),
                index == FAILED_FRAMES
            );
        }
    }

    #[test]
    fn unrelated_empty_outputs_and_successful_frames_do_not_confirm_a_stall() {
        let mut detector = HevcStallDetector::default();
        let now = Instant::now();
        let other_encoder = EMPTY_FRAME.replace("6097", "7000");
        let successful_frame = EMPTY_FRAME.replace("0xead39000-0, 0", "0xead39000-0, 752");
        for _ in 0..FAILED_FRAMES * 2 {
            assert!(!detector.observe_chunk(EMPTY_FRAME, now));
            assert!(!detector.observe_chunk(HARDWARE_ERROR, now));
            assert!(!detector.observe_chunk(&other_encoder, now));
            assert!(!detector.observe_chunk(&successful_frame, now));
        }
        for index in 0..FAILED_FRAMES {
            let now = now + FAILURE_WINDOW * u32::from(index) * 2;
            assert!(!detector.observe_chunk(HARDWARE_ERROR, now));
            assert!(!detector.observe_chunk(EMPTY_FRAME, now));
        }
    }
}
