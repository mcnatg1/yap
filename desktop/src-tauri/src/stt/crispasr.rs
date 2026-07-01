use std::path::Path;

use crate::stt::error::SttError;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
