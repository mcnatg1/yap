use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::stt::backend::{select_backend, BackendChoice, SttBackend};
use crate::stt::crispasr::CrispasrBackend;
use crate::stt::error::SttError;
use crate::stt::progress::{ProgressReporter, ProgressSink};
use crate::stt::python::PythonBackend;
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
        Self { code: error.code().to_string(), message: error.user_message().to_string() }
    }
}

pub struct SttState {
    pub sidecar: Arc<Mutex<CrispasrSidecar>>,
    fell_back: AtomicBool,
    transcribing: Arc<AtomicBool>,
}

impl SttState {
    pub fn new() -> Self {
        Self {
            sidecar: Arc::new(Mutex::new(CrispasrSidecar::new())),
            fell_back: AtomicBool::new(false),
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

    pub fn set_fell_back(&self, value: bool) {
        self.fell_back.store(value, Ordering::Relaxed);
    }

    pub fn fell_back(&self) -> bool {
        self.fell_back.load(Ordering::Relaxed)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineReadiness {
    Ready,
    Fallback,
    NotInstalled,
}

pub fn engine_readiness(engine_ready: bool, fell_back: bool) -> EngineReadiness {
    if fell_back {
        EngineReadiness::Fallback
    } else if engine_ready {
        EngineReadiness::Ready
    } else {
        EngineReadiness::NotInstalled
    }
}

pub fn engine_status_label(state: EngineReadiness) -> &'static str {
    match state {
        EngineReadiness::Ready => "Transcription engine ready",
        EngineReadiness::Fallback => "Using Python fallback",
        EngineReadiness::NotInstalled => "Transcription engine not installed yet",
    }
}

pub fn engine_status_for_binary(status: crate::stt::binary::BinaryInstallStatus) -> &'static str {
    match status {
        crate::stt::binary::BinaryInstallStatus::Installed => "Transcription engine ready",
        crate::stt::binary::BinaryInstallStatus::Downloadable => "Installing transcription engine…",
        crate::stt::binary::BinaryInstallStatus::Invalid => "Re-installing transcription engine…",
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
    root: PathBuf,
    paths: Vec<String>,
    language: &str,
) -> Result<Vec<TranscriptResult>, SttCommandError> {
    transcribe_paths_with_callbacks(state, root, paths, language, None, None, None)
}

pub fn transcribe_paths_with_callbacks(
    state: &SttState,
    root: PathBuf,
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

    let backend_env = std::env::var("YAP_STT_BACKEND").ok();
    let choice = select_backend(backend_env.as_deref());
    let crispasr = CrispasrBackend::new(Arc::clone(&state.sidecar));
    let python = PythonBackend::new(root);
    let mut fell_back = false;
    let total = paths.len();
    let mut results = Vec::with_capacity(total);

    for (index, path) in paths.iter().enumerate() {
        if let Some(ref sink) = progress {
            ProgressReporter::new(sink.clone(), path.clone(), index, total)
                .emit("starting", Some(0), "Preparing…");
        }

        let reporter = progress.as_ref().map(|sink| ProgressReporter::new(sink.clone(), path.clone(), index, total));
        let outcome = transcribe_one(
            choice,
            &crispasr,
            &python,
            &mut fell_back,
            path,
            language,
            reporter.as_ref(),
        );

        let result = match outcome {
            Ok(text) => {
                if let Some(ref reporter) = reporter {
                    reporter.emit("writing", Some(96), "Saving transcript…");
                }
                match write_sibling_txt(Path::new(path), &text) {
                    Ok(output) => TranscriptResult { input: path.clone(), output: output.display().to_string(), error: None },
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

    if fell_back {
        state.set_fell_back(true);
    }

    let succeeded = results.iter().filter(|result| result.error.is_none()).count();
    let failed = results.len().saturating_sub(succeeded);
    if let Some(handler) = on_batch_complete {
        handler(TranscribeBatchCompleteEvent { results: results.clone(), succeeded, failed });
    }

    Ok(results)
}

fn transcribe_one(
    choice: BackendChoice,
    crispasr: &CrispasrBackend,
    python: &PythonBackend,
    fell_back: &mut bool,
    path: &str,
    language: &str,
    reporter: Option<&ProgressReporter>,
) -> Result<String, SttError> {
    let audio = PathBuf::from(path);
    match choice {
        BackendChoice::Python => python.transcribe(&audio, language),
        BackendChoice::Crispasr => crispasr.transcribe_with_progress(&audio, language, reporter),
        BackendChoice::PreferCrispasr => match crispasr.transcribe_with_progress(&audio, language, reporter) {
            Ok(text) => Ok(text),
            Err(error) if is_engine_down(error) => {
                crate::stt::log_stt(&format!(
                    "crispasr unhealthy ({}); switching file to python fallback",
                    error.code()
                ));
                *fell_back = true;
                if let Some(reporter) = reporter {
                    reporter.emit("transcribing", Some(10), "Using Python fallback…");
                }
                python.transcribe(&audio, language)
            }
            Err(error) => Err(error),
        },
    }
}

pub fn dispatch<C, P>(
    choice: BackendChoice,
    crispasr: &C,
    python: &P,
    fell_back: &mut bool,
    paths: &[String],
    language: &str,
) -> Result<Vec<TranscriptResult>, SttCommandError>
where
    C: SttBackend,
    P: SttBackend,
{
    let outcomes = match choice {
        BackendChoice::Python => run_forced(python, paths, language),
        BackendChoice::Crispasr => run_forced(crispasr, paths, language),
        BackendChoice::PreferCrispasr => Ok(run_prefer(crispasr, python, fell_back, paths, language)),
    };
    let outcomes = outcomes.map_err(SttCommandError::from)?;
    Ok(finalize(paths, outcomes))
}

fn run_forced<B: SttBackend>(
    backend: &B,
    paths: &[String],
    language: &str,
) -> Result<Vec<Result<String, SttError>>, SttError> {
    let files: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let outcomes = backend.transcribe_batch(&files, language);
    let all_engine_down =
        !outcomes.is_empty() && outcomes.iter().all(|outcome| matches!(outcome, Err(error) if is_engine_down(*error)));
    if all_engine_down {
        let error = outcomes
            .iter()
            .find_map(|outcome| outcome.as_ref().err().copied())
            .unwrap_or(SttError::SidecarUnreachable);
        Err(error)
    } else {
        Ok(outcomes)
    }
}

fn run_prefer<C: SttBackend, P: SttBackend>(
    crispasr: &C,
    python: &P,
    fell_back: &mut bool,
    paths: &[String],
    language: &str,
) -> Vec<Result<String, SttError>> {
    let mut outcomes: Vec<Option<Result<String, SttError>>> = vec![None; paths.len()];
    let mut switch_index: Option<usize> = None;
    for (index, path) in paths.iter().enumerate() {
        let audio = PathBuf::from(path);
        match crispasr.transcribe(&audio, language) {
            Ok(text) => outcomes[index] = Some(Ok(text)),
            Err(error) if is_engine_down(error) => {
                crate::stt::log_stt(&format!(
                    "crispasr unhealthy ({}); switching remaining files to python fallback",
                    error.code()
                ));
                *fell_back = true;
                switch_index = Some(index);
                break;
            }
            Err(error) => outcomes[index] = Some(Err(error)),
        }
    }
    if let Some(start) = switch_index {
        let remaining: Vec<PathBuf> = paths[start..].iter().map(PathBuf::from).collect();
        for (offset, outcome) in python.transcribe_batch(&remaining, language).into_iter().enumerate() {
            outcomes[start + offset] = Some(outcome);
        }
    }
    outcomes.into_iter().map(|outcome| outcome.unwrap_or(Err(SttError::AudioDecode))).collect()
}

fn is_engine_down(error: SttError) -> bool {
    match error {
        SttError::SidecarCrash | SttError::SidecarUnreachable => true,
        SttError::ModelMissing
        | SttError::ModelCorrupt
        | SttError::BadLang
        | SttError::Oom
        | SttError::AudioDecode
        | SttError::Busy
        | SttError::Timeout => false,
    }
}

fn finalize(paths: &[String], outcomes: Vec<Result<String, SttError>>) -> Vec<TranscriptResult> {
    paths
        .iter()
        .zip(outcomes)
        .map(|(path, outcome)| {
            let audio = PathBuf::from(path);
            match outcome.and_then(|text| write_sibling_txt(&audio, &text)) {
                Ok(output) => TranscriptResult { input: path.clone(), output: output.display().to_string(), error: None },
                Err(error) => TranscriptResult {
                    input: path.clone(),
                    output: String::new(),
                    error: Some(error.code().to_string()),
                },
            }
        })
        .collect()
}

fn write_sibling_txt(audio: &Path, text: &str) -> Result<PathBuf, SttError> {
    let output = audio.with_extension("txt");
    std::fs::write(&output, format!("{text}\n")).map_err(|_| SttError::AudioDecode)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stt::backend::{BackendChoice, SttBackend};
    use crate::stt::error::SttError;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::Mutex;

    struct Scripted {
        queue: Mutex<VecDeque<Result<String, SttError>>>,
    }
    impl Scripted {
        fn new(items: Vec<Result<String, SttError>>) -> Self {
            Self { queue: Mutex::new(items.into_iter().collect()) }
        }
    }
    impl SttBackend for Scripted {
        fn transcribe(&self, _audio: &Path, _language: &str) -> Result<String, SttError> {
            self.queue.lock().unwrap().pop_front().unwrap_or(Err(SttError::AudioDecode))
        }
    }

    fn temp_paths(count: usize, tag: &str) -> Vec<String> {
        let base = std::env::temp_dir().join(format!("yap-dispatch-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        (0..count).map(|i| base.join(format!("clip{i}.wav")).display().to_string()).collect()
    }

    #[test]
    fn prefer_uses_crispasr_when_healthy_and_writes_sibling() {
        let paths = temp_paths(1, "healthy");
        let crispasr = Scripted::new(vec![Ok("crispasr text".into())]);
        let python = Scripted::new(vec![Ok("python text".into())]);
        let mut fell_back = false;
        let results =
            dispatch(BackendChoice::PreferCrispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_none());
        assert!(!fell_back);
        let sibling = std::path::PathBuf::from(&paths[0]).with_extension("txt");
        assert_eq!(std::fs::read_to_string(sibling).unwrap().trim(), "crispasr text");
    }

    #[test]
    fn prefer_falls_back_to_python_when_engine_down() {
        let paths = temp_paths(2, "fallback");
        let crispasr = Scripted::new(vec![Err(SttError::SidecarUnreachable)]);
        let python = Scripted::new(vec![Ok("py one".into()), Ok("py two".into())]);
        let mut fell_back = false;
        let results =
            dispatch(BackendChoice::PreferCrispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap();
        assert!(fell_back);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.error.is_none()));
    }

    #[test]
    fn per_file_audio_error_is_recorded_and_batch_continues() {
        let paths = temp_paths(2, "perfile");
        let crispasr = Scripted::new(vec![Err(SttError::AudioDecode), Ok("second".into())]);
        let python = Scripted::new(vec![]);
        let mut fell_back = false;
        let results =
            dispatch(BackendChoice::PreferCrispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap();
        assert_eq!(results[0].error.as_deref(), Some("AUDIO_DECODE"));
        assert!(results[1].error.is_none());
        assert!(!fell_back);
    }

    #[test]
    fn forced_crispasr_surfaces_when_engine_down() {
        let paths = temp_paths(1, "forced");
        let crispasr = Scripted::new(vec![Err(SttError::SidecarUnreachable)]);
        let python = Scripted::new(vec![]);
        let mut fell_back = false;
        let err =
            dispatch(BackendChoice::Crispasr, &crispasr, &python, &mut fell_back, &paths, "en").unwrap_err();
        assert_eq!(err.code, "SIDECAR_UNREACHABLE");
    }

    #[test]
    fn engine_status_labels_map_states() {
        assert_eq!(engine_status_label(engine_readiness(true, false)), "Transcription engine ready");
        assert_eq!(engine_status_label(engine_readiness(true, true)), "Using Python fallback");
        assert_eq!(engine_status_label(engine_readiness(false, false)), "Transcription engine not installed yet");
    }
}
