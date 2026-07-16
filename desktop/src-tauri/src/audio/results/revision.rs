use crate::audio::{
    evidence::{AlignedWord, ModelRevision, SpeakerTurn},
    session::SessionId,
};

use super::{
    validation::{
        validate_named_attribution_authority, validate_next_revision, validate_root_revision,
    },
    ResultAuthority, ResultRevisionError, ResultStatus, SpeakerResultRevision,
    TranscriptResultRevision,
};

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
        validate_named_attribution_authority(authority, &speaker_turns, &aligned_words)?;
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
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        let previous_result_sha256 = previous_result_sha256.into();
        validate_next_revision(
            self.revision,
            revision,
            &self.capture_sidecar_sha256,
            &capture_sidecar_sha256,
            &previous_result_sha256,
        )?;
        validate_named_attribution_authority(authority, &speaker_turns, &aligned_words)?;
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
        validate_named_attribution_authority(authority, &[], &aligned_words)?;
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

    #[allow(clippy::too_many_arguments)]
    pub fn next_revision(
        &self,
        revision: u64,
        authority: ResultAuthority,
        capture_sidecar_sha256: impl Into<String>,
        previous_result_sha256: impl Into<String>,
        status: ResultStatus,
        transcript: impl Into<String>,
        aligned_words: Vec<AlignedWord>,
        model_provenance: Vec<ModelRevision>,
    ) -> Result<Self, ResultRevisionError> {
        let capture_sidecar_sha256 = capture_sidecar_sha256.into();
        let previous_result_sha256 = previous_result_sha256.into();
        validate_next_revision(
            self.revision,
            revision,
            &self.capture_sidecar_sha256,
            &capture_sidecar_sha256,
            &previous_result_sha256,
        )?;
        validate_named_attribution_authority(authority, &[], &aligned_words)?;
        Ok(Self {
            session_id: self.session_id.clone(),
            revision,
            authority,
            capture_sidecar_sha256,
            previous_result_sha256: Some(previous_result_sha256),
            status,
            transcript: transcript.into(),
            aligned_words,
            model_provenance,
        })
    }
}
