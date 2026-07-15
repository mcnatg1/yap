use std::time::Duration;

use tokio::time::Instant;

use crate::stt::error::SttError;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub elapsed_ms: u128,
}

impl DownloadProgress {
    pub fn percent(self) -> Option<f32> {
        progress_metrics(self.downloaded_bytes, self.total_bytes, self.elapsed_ms).0
    }

    pub fn speed_mbps(self) -> Option<f32> {
        progress_metrics(self.downloaded_bytes, self.total_bytes, self.elapsed_ms).1
    }
}

#[derive(Debug)]
pub(super) struct BodyProgress {
    expected_bytes: u64,
    pub(super) downloaded_bytes: u64,
    timeout: Duration,
    deadline: Instant,
}

impl BodyProgress {
    pub(super) fn new(expected_bytes: u64, now: Instant, timeout: Duration) -> Self {
        Self {
            expected_bytes,
            downloaded_bytes: 0,
            timeout,
            deadline: now + timeout,
        }
    }

    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(super) fn record_chunk(&mut self, bytes: &[u8], now: Instant) -> Result<bool, SttError> {
        if bytes.is_empty() {
            return Ok(false);
        }
        let next = self
            .downloaded_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(SttError::ModelCorrupt)?;
        if next > self.expected_bytes {
            return Err(SttError::ModelCorrupt);
        }
        self.downloaded_bytes = next;
        self.deadline = now + self.timeout;
        Ok(true)
    }

    pub(super) fn finish(&self) -> Result<(), SttError> {
        (self.downloaded_bytes == self.expected_bytes)
            .then_some(())
            .ok_or(SttError::ModelCorrupt)
    }
}

pub(super) fn progress_metrics(
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    elapsed_ms: u128,
) -> (Option<f32>, Option<f32>) {
    let percent = total_bytes.and_then(|total| {
        (total > 0).then(|| ((downloaded_bytes as f32 / total as f32) * 100.0).clamp(0.0, 100.0))
    });
    let speed_mbps = (elapsed_ms > 0).then(|| {
        let elapsed_seconds = elapsed_ms as f32 / 1000.0;
        ((downloaded_bytes as f32 * 8.0) / elapsed_seconds) / 1_000_000.0
    });
    (percent, speed_mbps)
}
