#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttError {
    ModelMissing,
    ModelCorrupt,
    BadLang,
    Oom,
    AudioDecode,
    SidecarCrash,
    SidecarUnreachable,
    ServerUnavailable,
    FallbackDisabled,
    Busy,
    Timeout,
}

impl SttError {
    pub fn code(&self) -> &'static str {
        match self {
            SttError::ModelMissing => "MODEL_MISSING",
            SttError::ModelCorrupt => "MODEL_CORRUPT",
            SttError::BadLang => "BAD_LANG",
            SttError::Oom => "OOM",
            SttError::AudioDecode => "AUDIO_DECODE",
            SttError::SidecarCrash => "SIDECAR_CRASH",
            SttError::SidecarUnreachable => "SIDECAR_UNREACHABLE",
            SttError::ServerUnavailable => "SERVER_UNAVAILABLE",
            SttError::FallbackDisabled => "FALLBACK_DISABLED",
            SttError::Busy => "BUSY",
            SttError::Timeout => "TIMEOUT",
        }
    }

    pub fn user_message(&self) -> &'static str {
        match self {
            SttError::ModelMissing => "Local fallback model isn't installed yet.",
            SttError::ModelCorrupt => "Model file failed verification.",
            SttError::BadLang => "That language isn't supported.",
            SttError::Oom => "Ran out of memory while transcribing.",
            SttError::AudioDecode => "Couldn't read that audio file.",
            SttError::SidecarCrash => "Transcription engine crashed.",
            SttError::SidecarUnreachable => "Transcription engine didn't start.",
            SttError::ServerUnavailable => "Server transcription is unavailable.",
            SttError::FallbackDisabled => "Local fallback is disabled.",
            SttError::Busy => "Transcription is busy — try again in a moment.",
            SttError::Timeout => "Transcription timed out.",
        }
    }
}

impl std::fmt::Display for SttError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code(), self.user_message())
    }
}

impl std::error::Error for SttError {}

#[cfg(test)]
mod tests {
    use super::SttError;

    #[test]
    fn every_variant_has_stable_code_and_message() {
        let all = [
            SttError::ModelMissing,
            SttError::ModelCorrupt,
            SttError::BadLang,
            SttError::Oom,
            SttError::AudioDecode,
            SttError::SidecarCrash,
            SttError::SidecarUnreachable,
            SttError::ServerUnavailable,
            SttError::FallbackDisabled,
            SttError::Busy,
            SttError::Timeout,
        ];
        for error in all {
            assert!(!error.code().is_empty());
            assert!(!error.user_message().is_empty());
        }
        assert_eq!(SttError::SidecarUnreachable.code(), "SIDECAR_UNREACHABLE");
        assert_eq!(
            SttError::ModelCorrupt.user_message(),
            "Model file failed verification."
        );
        assert_eq!(
            SttError::Timeout.to_string(),
            "TIMEOUT: Transcription timed out."
        );
    }
}
