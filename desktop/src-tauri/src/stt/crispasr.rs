use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::stt::backend::SttBackend;
use crate::stt::error::SttError;
use crate::stt::progress::ProgressReporter;
use crate::stt::sidecar::{CrispasrSidecar, SidecarEndpoint};

pub const MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MIN_INFERENCE_TIMEOUT_SECS: u64 = 600;
const MAX_INFERENCE_TIMEOUT_SECS: u64 = 10_800;
const INFERENCE_TIMEOUT_BUFFER_SECS: u64 = 300;
const WAV_BYTES_PER_SECOND: u64 = 176_400;

pub fn estimate_audio_seconds(path: &Path, file_len: u64) -> u64 {
    if let Some(secs) = wav_duration_seconds(path) {
        return secs.max(1);
    }
    (file_len / WAV_BYTES_PER_SECOND).max(1)
}

pub fn inference_timeout_for(path: &Path) -> Duration {
    let file_len = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let audio_secs = estimate_audio_seconds(path, file_len);
    let timeout_secs = audio_secs
        .saturating_mul(3)
        .saturating_add(INFERENCE_TIMEOUT_BUFFER_SECS)
        .clamp(MIN_INFERENCE_TIMEOUT_SECS, MAX_INFERENCE_TIMEOUT_SECS);
    Duration::from_secs(timeout_secs)
}

fn wav_duration_seconds(path: &Path) -> Option<u64> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return None;
    }

    let mut byte_rate = None;
    let mut data_bytes = None;
    let mut chunk = [0u8; 8];
    while file.read_exact(&mut chunk).is_ok() {
        let chunk_id = &chunk[0..4];
        let chunk_size = u32::from_le_bytes(chunk[4..8].try_into().ok()?) as u64;
        if chunk_id == b"fmt " {
            let mut fmt = vec![0u8; chunk_size as usize];
            file.read_exact(&mut fmt).ok()?;
            if fmt.len() >= 16 {
                byte_rate = Some(u32::from_le_bytes(fmt[8..12].try_into().ok()?) as u64);
            }
        } else if chunk_id == b"data" {
            data_bytes = Some(chunk_size);
            break;
        } else {
            let mut remaining = chunk_size + (chunk_size % 2);
            let mut skip = [0u8; 4096];
            while remaining > 0 {
                let chunk = remaining.min(skip.len() as u64) as usize;
                let read = file.read(&mut skip[..chunk]).ok()?;
                if read == 0 {
                    break;
                }
                remaining -= read as u64;
            }
        }
    }

    let rate = byte_rate?;
    let bytes = data_bytes?;
    if rate == 0 {
        return None;
    }
    Some(bytes / rate)
}

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

    pub fn transcribe_with_progress(
        &self,
        audio: &Path,
        language: &str,
        reporter: Option<&ProgressReporter>,
    ) -> Result<String, SttError> {
        let _inflight = self.inflight.try_lock().map_err(|_| SttError::Busy)?;
        validate_audio_input(audio)?;

        let sidecar = Arc::clone(&self.sidecar);
        let ensure = || -> Result<SidecarEndpoint, SttError> {
            sidecar
                .lock()
                .map_err(|_| SttError::SidecarCrash)?
                .ensure_ready_with_progress(reporter)
        };
        let restart = || -> Result<SidecarEndpoint, SttError> {
            sidecar.lock().map_err(|_| SttError::SidecarCrash)?.restart()
        };

        if let Some(report) = reporter {
            report.emit("transcribing", Some(12), "Starting transcription…");
        }

        let result = run_with_retry(ensure, restart, |endpoint| {
            post_transcription_with_progress(endpoint, audio, language, reporter)
        });
        if result.is_ok() {
            if let Ok(mut guard) = self.sidecar.lock() {
                guard.mark_used();
            }
            if let Some(report) = reporter {
                report.emit("transcribing", Some(95), "Transcription complete.");
            }
        }
        result
    }
}

impl SttBackend for CrispasrBackend {
    fn transcribe(&self, audio: &Path, language: &str) -> Result<String, SttError> {
        self.transcribe_with_progress(audio, language, None)
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
    E: Fn() -> Result<SidecarEndpoint, SttError>,
    R: Fn() -> Result<SidecarEndpoint, SttError>,
    P: FnMut(&SidecarEndpoint) -> Result<String, SttError>,
{
    let endpoint = ensure()?;
    match post(&endpoint) {
        Ok(text) => Ok(text),
        Err(error) if is_sidecar_failure(error) => {
            let endpoint = restart()?;
            match post(&endpoint) {
                Ok(text) => Ok(text),
                Err(_) => Err(SttError::SidecarCrash),
            }
        }
        Err(error) => Err(error),
    }
}

fn transcribe_progress_estimate_secs(audio: &Path) -> u64 {
    let file_len = std::fs::metadata(audio).map(|meta| meta.len()).unwrap_or(0);
    let audio_secs = estimate_audio_seconds(audio, file_len);
    // Moonshine tiny is a degraded local fallback; keep a generous timeout for noisy clips.
    ((audio_secs as f64) * 1.55).max(30.0) as u64
}

fn post_transcription_with_progress(
    endpoint: &SidecarEndpoint,
    audio: &Path,
    language: &str,
    reporter: Option<&ProgressReporter>,
) -> Result<String, SttError> {
    let timeout = inference_timeout_for(audio);
    let audio_secs = estimate_audio_seconds(audio, std::fs::metadata(audio).map(|meta| meta.len()).unwrap_or(0));
    let estimated_secs = transcribe_progress_estimate_secs(audio);

    crate::stt::log_stt(&format!(
        "crispasr transcribe {} audio={}s est_cpu={}s timeout={}s",
        audio.display(),
        audio_secs,
        estimated_secs,
        timeout.as_secs()
    ));

    let done = Arc::new(AtomicBool::new(false));
    let progress_handle = reporter.map(|report| {
        let report = report.clone();
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let start = Instant::now();
            while !done.load(Ordering::Relaxed) {
                let elapsed = start.elapsed().as_secs();
                let ratio = (elapsed as f64 / estimated_secs as f64).clamp(0.0, 0.95);
                let percent = (15.0 + ratio * 75.0) as u8;
                let mins = elapsed / 60;
                let secs = elapsed % 60;
                report.emit(
                    "transcribing",
                    Some(percent),
                    &format!("Transcribing locally ({mins}m {secs:02}s)…"),
                );
                std::thread::sleep(Duration::from_secs(2));
            }
        })
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|_| SttError::SidecarUnreachable)?;
    let form = reqwest::blocking::multipart::Form::new()
        .file("file", audio)
        .map_err(|_| SttError::AudioDecode)?
        .text("language", language.to_string());
    let response = client
        .post(format!("{}/v1/audio/transcriptions", endpoint.url))
        .bearer_auth(&endpoint.api_key)
        .multipart(form)
        .send()
        .map_err(|err| if err.is_timeout() { SttError::Timeout } else { SttError::SidecarCrash })?;
    let status = response.status();
    let body = response.text().map_err(|_| SttError::SidecarCrash)?;

    done.store(true, Ordering::Relaxed);
    if let Some(handle) = progress_handle {
        let _ = handle.join();
    }

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

    fn endpoint(url: &str) -> SidecarEndpoint {
        SidecarEndpoint {
            url: url.to_string(),
            api_key: "secret".to_string(),
        }
    }

    #[test]
    fn parse_reads_text_and_ignores_unknown_fields() {
        let body = r#"{"text":"hello world","segments":[{"start":0.0}],"backend":"moonshine-streaming","extra":42}"#;
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
    fn transcribe_progress_estimate_uses_cpu_realtime_factor() {
        let dir = std::env::temp_dir().join(format!("yap-progress-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.wav");
        std::fs::write(&path, vec![0u8; 176_400 * 120]).unwrap();
        let est = transcribe_progress_estimate_secs(&path);
        assert!(est >= 120);
        assert!(est <= 240);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inference_timeout_scales_with_estimated_duration() {
        let dir = std::env::temp_dir().join(format!("yap-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.wav");
        std::fs::write(&path, vec![0u8; 176_400 * 120]).unwrap();
        let timeout = inference_timeout_for(&path);
        assert!(timeout.as_secs() >= 600);
        assert!(timeout.as_secs() <= 10_800);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn retry_succeeds_on_first_post() {
        let calls = Cell::new(0);
        let out = run_with_retry(
            || Ok(endpoint("url")),
            || Ok(endpoint("url2")),
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
            || Ok(endpoint("url")),
            || Ok(endpoint("url2")),
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
            || Ok(endpoint("url")),
            || Ok(endpoint("url2")),
            |_url| Err(SttError::SidecarUnreachable),
        );
        assert_eq!(out.unwrap_err(), SttError::SidecarCrash);
    }

    #[test]
    fn retry_propagates_non_sidecar_errors_without_restart() {
        let restarted = Cell::new(false);
        let out = run_with_retry(
            || Ok(endpoint("url")),
            || {
                restarted.set(true);
                Ok(endpoint("url2"))
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
