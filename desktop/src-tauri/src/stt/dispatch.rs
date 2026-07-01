use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::stt::backend::{select_backend, BackendChoice, SttBackend};
use crate::stt::crispasr::CrispasrBackend;
use crate::stt::error::SttError;
use crate::stt::python::PythonBackend;
use crate::stt::sidecar::CrispasrSidecar;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResult {
    pub input: String,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
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
}

impl SttState {
    pub fn new() -> Self {
        Self { sidecar: Arc::new(Mutex::new(CrispasrSidecar::new())), fell_back: AtomicBool::new(false) }
    }

    pub fn set_fell_back(&self, value: bool) {
        self.fell_back.store(value, Ordering::Relaxed);
    }

    pub fn fell_back(&self) -> bool {
        self.fell_back.load(Ordering::Relaxed)
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

pub fn engine_ready() -> bool {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let binary_ok = crate::stt::sidecar::resolve_binary(|key| std::env::var(key).ok(), &exe_dir).is_ok();
    let model_ok = crate::stt::pin::load_pin()
        .map(|pin| crate::stt::model::models_dir().join(pin.gguf_file).exists())
        .unwrap_or(false);
    binary_ok && model_ok
}

pub fn transcribe_paths(
    state: &SttState,
    root: PathBuf,
    paths: Vec<String>,
    language: &str,
) -> Result<Vec<TranscriptResult>, SttCommandError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let backend_env = std::env::var("YAP_STT_BACKEND").ok();
    let choice = select_backend(backend_env.as_deref());
    let crispasr = CrispasrBackend::new(Arc::clone(&state.sidecar));
    let python = PythonBackend::new(root);
    let mut fell_back = false;
    let result = dispatch(choice, &crispasr, &python, &mut fell_back, &paths, language);
    if fell_back {
        state.set_fell_back(true);
    }
    result
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
