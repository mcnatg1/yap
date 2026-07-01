use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::stt::backend::SttBackend;
use crate::stt::error::SttError;
use crate::stt::sidecar::CrispasrSidecar;

pub const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn parse_transcription_json(body: &str) -> Result<String, SttError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|_| SttError::SidecarCrash)?;
    match value.get("text").and_then(serde_json::Value::as_str) {
        Some(text) => Ok(text.to_string()),
        None => Err(SttError::SidecarCrash),
    }
}

pub fn classify_response(status: u16, body: &str) -> SttError {
    let lower = body.to_lowercase();
    if status == 408 {
        SttError::Timeout
    } else if lower.contains("language") {
        SttError::BadLang
    } else if lower.contains("out of memory") || lower.contains("oom") {
        SttError::Oom
    } else if lower.contains("decode") || lower.contains("audio") {
        SttError::AudioDecode
    } else {
        SttError::SidecarCrash
    }
}

pub fn check_audio_size(len: u64, max: u64) -> Result<(), SttError> {
    if len == 0 || len > max {
        Err(SttError::AudioDecode)
    } else {
        Ok(())
    }
}

pub fn validate_audio_input(path: &Path) -> Result<(), SttError> {
    let metadata = std::fs::metadata(path).map_err(|_| SttError::AudioDecode)?;
    if !metadata.is_file() {
        return Err(SttError::AudioDecode);
    }
    check_audio_size(metadata.len(), MAX_AUDIO_BYTES)
}

pub struct CrispasrBackend {
    sidecar: Arc<Mutex<CrispasrSidecar>>,
    inflight: Arc<Mutex<()>>,
}

impl CrispasrBackend {
    pub fn new(sidecar: Arc<Mutex<CrispasrSidecar>>) -> Self {
        Self { sidecar, inflight: Arc::new(Mutex::new(())) }
    }
}

impl SttBackend for CrispasrBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError> {
        let _inflight = self.inflight.try_lock().map_err(|_| SttError::Busy)?;
        validate_audio_input(audio)?;

        let sidecar = Arc::clone(&self.sidecar);
        let ensure = || -> Result<String, SttError> {
            sidecar.lock().map_err(|_| SttError::SidecarCrash)?.ensure_ready()
        };
        let restart = || -> Result<String, SttError> {
            sidecar.lock().map_err(|_| SttError::SidecarCrash)?.restart()
        };

        let result = run_with_retry(ensure, restart, |url| post_transcription(url, audio, language));
        if result.is_ok() {
            if let Ok(mut guard) = self.sidecar.lock() {
                guard.mark_used();
            }
        }
        result
    }
}

fn is_sidecar_failure(error: SttError) -> bool {
    match error {
        SttError::SidecarCrash | SttError::SidecarUnreachable | SttError::Timeout => true,
        SttError::ModelMissing
        | SttError::ModelCorrupt
        | SttError::BadLang
        | SttError::Oom
        | SttError::AudioDecode
        | SttError::Busy => false,
    }
}

fn run_with_retry<E, R, P>(ensure: E, restart: R, mut post: P) -> Result<String, SttError>
where
    E: Fn() -> Result<String, SttError>,
    R: Fn() -> Result<String, SttError>,
    P: FnMut(&str) -> Result<String, SttError>,
{
    let url = ensure()?;
    match post(&url) {
        Ok(text) => Ok(text),
        Err(error) if is_sidecar_failure(error) => {
            let url = restart()?;
            match post(&url) {
                Ok(text) => Ok(text),
                Err(_) => Err(SttError::SidecarCrash),
            }
        }
        Err(error) => Err(error),
    }
}

fn post_transcription(base_url: &str, audio: &Path, language: &str) -> Result<String, SttError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|_| SttError::SidecarUnreachable)?;
    let form = reqwest::blocking::multipart::Form::new()
        .file("file", audio)
        .map_err(|_| SttError::AudioDecode)?
        .text("language", language.to_string());
    let response = client
        .post(format!("{base_url}/v1/audio/transcriptions"))
        .multipart(form)
        .send()
        .map_err(|err| if err.is_timeout() { SttError::Timeout } else { SttError::SidecarCrash })?;
    let status = response.status();
    let body = response.text().map_err(|_| SttError::SidecarCrash)?;
    if status.is_success() {
        parse_transcription_json(&body)
    } else {
        Err(classify_response(status.as_u16(), &body))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::stt::sidecar::CrispasrSidecar;

    #[test]
    fn parse_reads_text_and_ignores_unknown_fields() {
        let body = r#"{"text":"hello world","segments":[{"start":0.0}],"backend":"cohere","extra":42}"#;
        assert_eq!(parse_transcription_json(body).unwrap(), "hello world");
    }

    #[test]
    fn parse_accepts_empty_text() {
        assert_eq!(parse_transcription_json(r#"{"text":""}"#).unwrap(), "");
    }

    #[test]
    fn parse_rejects_missing_text_and_bad_json() {
        assert_eq!(parse_transcription_json(r#"{"segments":[]}"#).unwrap_err(), SttError::SidecarCrash);
        assert_eq!(parse_transcription_json("not json").unwrap_err(), SttError::SidecarCrash);
    }

    #[test]
    fn classify_response_maps_status_and_body() {
        assert_eq!(classify_response(408, ""), SttError::Timeout);
        assert_eq!(classify_response(400, "unsupported language code"), SttError::BadLang);
        assert_eq!(classify_response(500, "ggml out of memory"), SttError::Oom);
        assert_eq!(classify_response(500, "failed to decode audio"), SttError::AudioDecode);
        assert_eq!(classify_response(500, "panic"), SttError::SidecarCrash);
    }

    #[test]
    fn check_audio_size_rejects_empty_and_oversized() {
        assert!(check_audio_size(1, MAX_AUDIO_BYTES).is_ok());
        assert_eq!(check_audio_size(0, MAX_AUDIO_BYTES).unwrap_err(), SttError::AudioDecode);
        assert_eq!(check_audio_size(MAX_AUDIO_BYTES + 1, MAX_AUDIO_BYTES).unwrap_err(), SttError::AudioDecode);
    }

    #[test]
    fn retry_succeeds_on_first_post() {
        let calls = Cell::new(0);
        let out = run_with_retry(
            || Ok("url".to_string()),
            || Ok("url2".to_string()),
            |_url| {
                calls.set(calls.get() + 1);
                Ok("hi".to_string())
            },
        );
        assert_eq!(out.unwrap(), "hi");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn retry_restarts_then_succeeds() {
        let calls = Cell::new(0);
        let out = run_with_retry(
            || Ok("url".to_string()),
            || Ok("url2".to_string()),
            |_url| {
                let n = calls.get();
                calls.set(n + 1);
                if n == 0 {
                    Err(SttError::SidecarCrash)
                } else {
                    Ok("recovered".to_string())
                }
            },
        );
        assert_eq!(out.unwrap(), "recovered");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn retry_gives_up_after_second_failure() {
        let out = run_with_retry(
            || Ok("url".to_string()),
            || Ok("url2".to_string()),
            |_url| Err(SttError::SidecarUnreachable),
        );
        assert_eq!(out.unwrap_err(), SttError::SidecarCrash);
    }

    #[test]
    fn retry_propagates_non_sidecar_errors_without_restart() {
        let restarted = Cell::new(false);
        let out = run_with_retry(
            || Ok("url".to_string()),
            || {
                restarted.set(true);
                Ok("url2".to_string())
            },
            |_url| Err(SttError::AudioDecode),
        );
        assert_eq!(out.unwrap_err(), SttError::AudioDecode);
        assert!(!restarted.get());
    }

    #[test]
    fn transcribe_returns_busy_when_a_request_is_in_flight() {
        let backend = CrispasrBackend::new(Arc::new(Mutex::new(CrispasrSidecar::new())));
        let _held = backend.inflight.lock().unwrap();
        let err = backend.transcribe(Path::new("C:/clips/a.wav"), "en").unwrap_err();
        assert_eq!(err, SttError::Busy);
    }
}
