use crate::audio::{
    evidence::{AlignedWord, ModelRevision, SpeakerTurn},
    session::SessionId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultAuthority {
    LocalProvisional,
    LocalReconciled,
    ServerAuthoritative,
    UserCorrected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResultRevision {
    pub session_id: SessionId,
    pub revision: u64,
    pub authority: ResultAuthority,
    pub capture_sidecar_sha256: String,
    pub previous_result_sha256: Option<String>,
    pub status: ResultStatus,
    pub transcript: String,
    pub aligned_words: Vec<AlignedWord>,
    pub model_provenance: Vec<ModelRevision>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerResultRevision {
    pub session_id: SessionId,
    pub revision: u64,
    pub authority: ResultAuthority,
    pub capture_sidecar_sha256: String,
    pub previous_result_sha256: Option<String>,
    pub status: ResultStatus,
    pub speaker_turns: Vec<SpeakerTurn>,
    pub aligned_words: Vec<AlignedWord>,
    pub model_provenance: Vec<ModelRevision>,
}

impl SpeakerResultRevision {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: Option<String>,
        status: ResultStatus,
        speaker_turns: Vec<SpeakerTurn>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        validate_root_revision(
            revision,
            &capture_sidecar_sha256,
            previous_result_sha256.as_deref(),
        )?;
        Ok(Self {
            session_id,
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256,
            status,
            speaker_turns,
            aligned_words,
            model_provenance,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn next_revision(
        &self,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: impl Into<String>,
        status: ResultStatus,
        speaker_turns: Vec<SpeakerTurn>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        if revision
            != self
                .revision
                .checked_add(1)
                .ok_or(ResultRevisionError::RevisionOverflow)?
        {
            return Err(ResultRevisionError::NonMonotonicRevision);
        }
        let previous_result_sha256 = previous_result_sha256.into();
        if previous_result_sha256.is_empty() {
            return Err(ResultRevisionError::MissingPreviousResultHash);
        }
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        if capture_sidecar_sha256.is_empty() {
            return Err(ResultRevisionError::MissingCaptureHash);
        }
        Ok(Self {
            session_id: self.session_id.clone(),
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256: Some(previous_result_sha256),
            status,
            speaker_turns,
            aligned_words,
            model_provenance,
        })
    }
}

impl TranscriptResultRevision {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: Option<String>,
        status: ResultStatus,
        transcript: impl Into<String>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        validate_root_revision(
            revision,
            &capture_sidecar_sha256,
            previous_result_sha256.as_deref(),
        )?;
        Ok(Self {
            session_id,
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256,
            status,
            transcript: transcript.into(),
            aligned_words,
            model_provenance,
        })
    }
}

fn validate_root_revision(
    revision: u64,
    capture_sidecar_sha256: &str,
    previous_result_sha256: Option<&str>,
) -> Result<(), ResultRevisionError> {
    if capture_sidecar_sha256.is_empty() {
        return Err(ResultRevisionError::MissingCaptureHash);
    }
    if revision != 1 {
        return Err(ResultRevisionError::NonMonotonicRevision);
    }
    if previous_result_sha256.is_some() {
        return Err(ResultRevisionError::UnexpectedPreviousResultHash);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultRevisionError {
    MissingCaptureHash,
    MissingPreviousResultHash,
    UnexpectedPreviousResultHash,
    NonMonotonicRevision,
    RevisionOverflow,
}

impl std::fmt::Display for ResultRevisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ResultRevisionError {}

#[cfg(test)]
mod tests {
    use crate::audio::{
        evidence::{ClientSpeakerAttribution, EvidenceQuality, ModelRevision, SpeakerTurn},
        session::SessionId,
    };

    use super::{ResultAuthority, ResultStatus, SpeakerResultRevision};

    fn turn() -> SpeakerTurn {
        SpeakerTurn::new(
            "turn-1",
            0,
            20,
            ClientSpeakerAttribution::unknown(),
            Some(0.8),
        )
        .unwrap()
    }

    #[test]
    fn result_revisions_require_capture_hash_and_monotonic_revision() {
        let session_id = SessionId::new("s-result").unwrap();
        let model = ModelRevision::new("speaker-model", "r1", "calibration-r1").unwrap();

        assert!(SpeakerResultRevision::new(
            session_id.clone(),
            1,
            ResultAuthority::LocalProvisional,
            "",
            None,
            ResultStatus::Complete,
            vec![turn()],
            Vec::new(),
            vec![model.clone()],
        )
        .is_err());

        let first = SpeakerResultRevision::new(
            session_id,
            1,
            ResultAuthority::LocalProvisional,
            "capture-sha256",
            None,
            ResultStatus::Complete,
            vec![turn()],
            Vec::new(),
            vec![model],
        )
        .unwrap();

        assert!(first
            .next_revision(
                1,
                ResultAuthority::LocalReconciled,
                "capture-sha256",
                "previous-result-sha256",
                ResultStatus::Complete,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .is_err());
        assert!(first
            .next_revision(
                2,
                ResultAuthority::LocalReconciled,
                "capture-sha256",
                "",
                ResultStatus::Complete,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .is_err());
        assert!(first
            .next_revision(
                2,
                ResultAuthority::LocalReconciled,
                "capture-sha256",
                "previous-result-sha256",
                ResultStatus::Complete,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
            .is_ok());
    }

    #[test]
    fn evidence_and_result_json_contains_no_embedding_or_exemplar_values() {
        let evidence = crate::audio::evidence::SpeakerEvidence::new(
            crate::audio::session::TrackId::new("mic-1").unwrap(),
            0,
            20,
            Some("slot-1".into()),
            ModelRevision::new("speaker-model", "r1", "calibration-r1").unwrap(),
            EvidenceQuality::Clean,
            Some(0.8),
        )
        .unwrap();
        let result = SpeakerResultRevision::new(
            SessionId::new("s-result").unwrap(),
            1,
            ResultAuthority::LocalProvisional,
            "capture-sha256",
            None,
            ResultStatus::Partial,
            vec![turn()],
            Vec::new(),
            vec![ModelRevision::new("speaker-model", "r1", "calibration-r1").unwrap()],
        )
        .unwrap();

        let serialized = format!(
            "{}{}",
            serde_json::to_string(&evidence).unwrap(),
            serde_json::to_string(&result).unwrap()
        );
        assert!(!serialized.contains("embedding"));
        assert!(!serialized.contains("exemplar"));
    }
}
