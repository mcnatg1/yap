use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::stt::crispasr::CrispasrBackend;
use crate::stt::error::SttError;
use crate::stt::progress::{ProgressReporter, ProgressSink};
use crate::stt::sidecar::CrispasrSidecar;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeFileCompleteEvent {
    pub path: String,
    pub index: usize,
    pub total: usize,
    pub result: TranscriptResult,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeBatchCompleteEvent {
    pub results: Vec<TranscriptResult>,
    pub succeeded: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResult {
    pub input: String,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

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
    pub sidecar: Arc<Mutex<CrispasrSidecar>>,
    transcribing: Arc<AtomicBool>,
}

impl SttState {
    pub fn new() -> Self {
        Self {
            sidecar: Arc::new(Mutex::new(CrispasrSidecar::new())),
            transcribing: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn transcribing_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.transcribing)
    }

    pub fn set_transcribing(&self, value: bool) {
        self.transcribing.store(value, Ordering::Relaxed);
        if value {
            if let Ok(mut sidecar) = self.sidecar.lock() {
                sidecar.mark_used();
            }
        }
    }

    pub fn is_transcribing(&self) -> bool {
        self.transcribing.load(Ordering::Relaxed)
    }

    pub fn reset_sidecar(&self) {
        if let Ok(mut sidecar) = self.sidecar.lock() {
            sidecar.shutdown();
        }
    }
}

impl Default for SttState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn engine_status_for_binary(status: crate::stt::binary::BinaryInstallStatus) -> &'static str {
    match status {
        crate::stt::binary::BinaryInstallStatus::Installed => "Transcription engine ready",
        crate::stt::binary::BinaryInstallStatus::Downloadable => {
            "Installing transcription engine..."
        }
        crate::stt::binary::BinaryInstallStatus::Invalid => "Re-installing transcription engine...",
        crate::stt::binary::BinaryInstallStatus::Unsupported => {
            "Transcription engine requires manual install"
        }
    }
}

pub fn engine_ready() -> bool {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let binary_ok = crate::stt::binary::binary_install_status(&exe_dir)
        .map(|status| {
            matches!(
                status,
                crate::stt::binary::BinaryInstallStatus::Installed
                    | crate::stt::binary::BinaryInstallStatus::Downloadable
            )
        })
        .unwrap_or(false);
    crate::stt::pin::load_pin().is_ok() && binary_ok
}

pub fn engine_binary_status_label(status: crate::stt::binary::BinaryInstallStatus) -> &'static str {
    status.label()
}

pub fn transcribe_paths(
    state: &SttState,
    paths: Vec<String>,
    language: &str,
) -> Result<Vec<TranscriptResult>, SttCommandError> {
    transcribe_paths_with_callbacks(state, paths, language, None, None, None)
}

pub fn transcribe_paths_with_callbacks(
    state: &SttState,
    paths: Vec<String>,
    language: &str,
    progress: Option<ProgressSink>,
    on_file_complete: Option<Arc<dyn Fn(TranscribeFileCompleteEvent) + Send + Sync>>,
    on_batch_complete: Option<Arc<dyn Fn(TranscribeBatchCompleteEvent) + Send + Sync>>,
) -> Result<Vec<TranscriptResult>, SttCommandError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    state.set_transcribing(true);
    struct TranscribingGuard<'a>(&'a SttState);
    impl Drop for TranscribingGuard<'_> {
        fn drop(&mut self) {
            self.0.set_transcribing(false);
        }
    }
    let _guard = TranscribingGuard(state);

    let backend = CrispasrBackend::new(Arc::clone(&state.sidecar));
    let total = paths.len();
    let mut results = Vec::with_capacity(total);

    for (index, path) in paths.iter().enumerate() {
        if let Some(ref sink) = progress {
            ProgressReporter::new(sink.clone(), path.clone(), index, total).emit(
                "starting",
                Some(0),
                "Preparing...",
            );
        }

        let reporter = progress
            .as_ref()
            .map(|sink| ProgressReporter::new(sink.clone(), path.clone(), index, total));
        let result =
            match backend.transcribe_with_progress(Path::new(path), language, reporter.as_ref()) {
                Ok(text) => {
                    if let Some(ref reporter) = reporter {
                        reporter.emit("writing", Some(96), "Saving transcript...");
                    }
                    match write_sibling_txt(Path::new(path), &text) {
                        Ok(output) => TranscriptResult {
                            input: path.clone(),
                            output: output.display().to_string(),
                            error: None,
                        },
                        Err(error) => TranscriptResult {
                            input: path.clone(),
                            output: String::new(),
                            error: Some(error.code().to_string()),
                        },
                    }
                }
                Err(error) => TranscriptResult {
                    input: path.clone(),
                    output: String::new(),
                    error: Some(error.code().to_string()),
                },
            };

        if let Some(ref reporter) = reporter {
            if result.error.is_none() {
                reporter.emit("done", Some(100), "Transcript saved");
            }
        }

        if let Some(ref handler) = on_file_complete {
            handler(TranscribeFileCompleteEvent {
                path: path.clone(),
                index,
                total,
                result: result.clone(),
            });
        }
        results.push(result);
    }

    let succeeded = results
        .iter()
        .filter(|result| result.error.is_none())
        .count();
    let failed = results.len().saturating_sub(succeeded);
    if let Some(handler) = on_batch_complete {
        handler(TranscribeBatchCompleteEvent {
            results: results.clone(),
            succeeded,
            failed,
        });
    }

    Ok(results)
}

fn write_sibling_txt(audio: &Path, text: &str) -> Result<PathBuf, SttError> {
    let output = audio.with_extension("txt");
    std::fs::write(&output, format!("{text}\n")).map_err(|_| SttError::AudioDecode)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_batch_returns_without_starting() {
        let state = SttState::new();
        let results = transcribe_paths(&state, Vec::new(), "en").unwrap();
        assert!(results.is_empty());
        assert!(!state.is_transcribing());
    }

    #[test]
    fn write_sibling_txt_writes_next_to_audio() {
        let dir = std::env::temp_dir().join(format!("yap-dispatch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("clip.wav");
        std::fs::write(&audio, b"audio").unwrap();
        let out = write_sibling_txt(&audio, "hello").unwrap();
        assert_eq!(out, dir.join("clip.txt"));
        assert_eq!(std::fs::read_to_string(out).unwrap(), "hello\n");
        std::fs::remove_dir_all(&dir).ok();
    }
}
