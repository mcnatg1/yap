use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::stt::error::SttError;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttCommandError {
    pub code: String,
    pub message: String,
}

impl From<SttError> for SttCommandError {
    fn from(error: SttError) -> Self {
        Self {
            code: error.code().to_string(),
            message: error.user_message().to_string(),
        }
    }
}

pub struct SttState {
    transcribing: Arc<AtomicBool>,
}

impl SttState {
    pub fn new() -> Self {
        Self {
            transcribing: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_transcribing(&self, value: bool) {
        self.transcribing.store(value, Ordering::Relaxed);
    }

    pub fn is_transcribing(&self) -> bool {
        self.transcribing.load(Ordering::Relaxed)
    }
}

impl Default for SttState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_tracks_busy_without_sidecar_runtime() {
        let state = SttState::new();
        assert!(!state.is_transcribing());
        state.set_transcribing(true);
        assert!(state.is_transcribing());
        state.set_transcribing(false);
        assert!(!state.is_transcribing());
    }
}
