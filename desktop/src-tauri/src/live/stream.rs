use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::Value;

use crate::stt::error::SttError;

const STREAM_BACKEND: &str = "moonshine-streaming";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Partial(String),
    Final(String),
}

#[derive(Debug)]
pub struct LiveStreamProcess {
    child: Child,
}

impl LiveStreamProcess {
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LiveStreamProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPaths {
    pub binary: PathBuf,
    pub model: PathBuf,
    pub punc_model: PathBuf,
}

pub fn build_stream_args(
    model: &Path,
    punc_model: &Path,
    gpu: &crate::stt::gpu::GpuStatus,
) -> Vec<String> {
    let mut args = vec![
        "--stream".to_string(),
        "--stream-json".to_string(),
        "--backend".to_string(),
        STREAM_BACKEND.to_string(),
        "-m".to_string(),
        model.to_string_lossy().to_string(),
        "-l".to_string(),
        "en".to_string(),
        "--punc-model".to_string(),
        punc_model.to_string_lossy().to_string(),
    ];
    if gpu.layers > 0 {
        args.push("--gpu-backend".to_string());
        args.push("auto".to_string());
    } else {
        args.push("-ng".to_string());
    }
    args
}

pub fn parse_stream_event(line: &str) -> Option<StreamEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    let text = value.get("text")?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }

    let kinds = ["type", "event", "status"]
        .into_iter()
        .filter_map(|key| value.get(key).and_then(Value::as_str))
        .map(str::to_lowercase)
        .collect::<Vec<_>>();

    if kinds.iter().any(|kind| kind.contains("final")) {
        Some(StreamEvent::Final(text.to_string()))
    } else if kinds.iter().any(|kind| kind.contains("partial")) || kinds.is_empty() {
        Some(StreamEvent::Partial(text.to_string()))
    } else {
        None
    }
}

pub fn resolve_stream_paths() -> Result<StreamPaths, SttError> {
    if !crate::stt::settings::local_fallback_enabled() {
        crate::stt::log_stt("live stream: local fallback disabled");
        return Err(SttError::FallbackDisabled);
    }

    let exe_dir = current_exe_dir();
    let binary = crate::stt::binary::resolve_for_spawn(&exe_dir)?;
    let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;
    if !crate::stt::model::is_installed(&pin) {
        crate::stt::log_stt("live stream: local fallback model missing");
        return Err(SttError::ModelMissing);
    }

    let model_dir = crate::stt::model::models_dir();
    Ok(StreamPaths {
        binary,
        model: model_dir.join(&pin.gguf_file),
        punc_model: model_dir.join(&pin.punc_file),
    })
}

pub fn spawn_stream_child() -> Result<LiveStreamProcess, SttError> {
    let paths = resolve_stream_paths()?;
    let gpu = crate::stt::gpu::GpuStatus::resolve();
    spawn_stream_child_with_paths(&paths, &gpu)
}

pub fn spawn_stream_child_with_paths(
    paths: &StreamPaths,
    gpu: &crate::stt::gpu::GpuStatus,
) -> Result<LiveStreamProcess, SttError> {
    spawn_child(&paths.binary, &paths.model, &paths.punc_model, gpu)
}

fn spawn_child(
    binary: &Path,
    model: &Path,
    punc_model: &Path,
    gpu: &crate::stt::gpu::GpuStatus,
) -> Result<LiveStreamProcess, SttError> {
    let stderr_path = crate::stt::sidecar_stderr_log_path();
    if let Some(parent) = stderr_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let stderr_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .map_err(|_| SttError::SidecarUnreachable)?;
    let args = build_stream_args(model, punc_model, gpu);
    crate::stt::log_stt(&format!(
        "spawning live stream stderr_log={} binary={} args={:?}",
        stderr_path.display(),
        binary.display(),
        args
    ));

    let mut command = Command::new(binary);
    command.args(args);
    command.env_clear();
    command.envs(crate::stt::sidecar::sidecar_env(std::env::vars()));
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::from(stderr_file));
    crate::stt::hide_child_console(&mut command);

    let child = command.spawn().map_err(|err| {
        crate::stt::log_stt(&format!("live stream spawn failed: {err}"));
        SttError::SidecarUnreachable
    })?;
    Ok(LiveStreamProcess { child })
}

fn current_exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_args_keep_punctuation_and_gpu_choice() {
        let gpu = crate::stt::gpu::GpuStatus {
            available: true,
            adapter_name: Some("test gpu".into()),
            preference: crate::stt::gpu::GpuPreference::Auto,
            layers: 99,
        };
        let args = build_stream_args(
            std::path::Path::new("C:/models/moonshine.gguf"),
            std::path::Path::new("C:/models/punc.gguf"),
            &gpu,
        );
        assert!(args.contains(&"--stream".to_string()));
        assert!(args.contains(&"--stream-json".to_string()));
        assert!(args.contains(&"--punc-model".to_string()));
        assert!(!args.contains(&"--no-punctuation".to_string()));
        assert!(args.contains(&"--gpu-backend".to_string()));
    }

    #[test]
    fn stream_args_disable_gpu_when_layers_are_zero() {
        let gpu = crate::stt::gpu::GpuStatus {
            available: false,
            adapter_name: None,
            preference: crate::stt::gpu::GpuPreference::Cpu,
            layers: 0,
        };
        let args = build_stream_args(
            std::path::Path::new("C:/models/moonshine.gguf"),
            std::path::Path::new("C:/models/punc.gguf"),
            &gpu,
        );
        assert!(args.contains(&"-ng".to_string()));
        assert!(!args.contains(&"--gpu-backend".to_string()));
        assert!(!args.contains(&"--no-punctuation".to_string()));
    }

    #[test]
    fn parses_partial_and_final_events() {
        assert_eq!(
            parse_stream_event(r#"{"type":"partial","text":"hello"}"#),
            Some(StreamEvent::Partial("hello".into()))
        );
        assert_eq!(
            parse_stream_event(r#"{"event":"final","text":"hello."}"#),
            Some(StreamEvent::Final("hello.".into()))
        );
        assert_eq!(parse_stream_event("not json"), None);
    }

    #[test]
    fn parses_untyped_text_as_partial_and_ignores_empty_text() {
        assert_eq!(
            parse_stream_event(r#"{"text":"still listening"}"#),
            Some(StreamEvent::Partial("still listening".into()))
        );
        assert_eq!(
            parse_stream_event(r#"{"type":"partial","text":"   "}"#),
            None
        );
        assert_eq!(parse_stream_event(r#"{"type":"partial"}"#), None);
    }

    #[test]
    fn parser_accepts_status_or_event_kind() {
        assert_eq!(
            parse_stream_event(r#"{"status":"utterance_final","text":"done."}"#),
            Some(StreamEvent::Final("done.".into()))
        );
        assert_eq!(
            parse_stream_event(r#"{"type":"unknown","event":"partial_update","text":"do"}"#),
            Some(StreamEvent::Partial("do".into()))
        );
    }
}
