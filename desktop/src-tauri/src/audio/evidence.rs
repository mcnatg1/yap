use crate::audio::session::TrackId;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRevision {
    model_id: String,
    revision: String,
    calibration_revision: String,
}

impl ModelRevision {
    pub fn new(
        model_id: impl Into<String>,
        revision: impl Into<String>,
        calibration_revision: impl Into<String>,
    ) -> Result<Self, EvidenceError> {
        let value = Self {
            model_id: model_id.into(),
            revision: revision.into(),
            calibration_revision: calibration_revision.into(),
        };
        if value.model_id.is_empty()
            || value.revision.is_empty()
            || value.calibration_revision.is_empty()
        {
            return Err(EvidenceError::MissingProvenance);
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceQuality {
    Clean,
    Weak,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerEvidence {
    track_id: TrackId,
    start_ms: u64,
    end_ms: u64,
    local_slot_id: Option<String>,
    model: ModelRevision,
    quality: EvidenceQuality,
    confidence: Option<f32>,
}

impl SpeakerEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        track_id: TrackId,
        start_ms: u64,
        end_ms: u64,
        local_slot_id: Option<String>,
        model: ModelRevision,
        quality: EvidenceQuality,
        confidence: Option<f32>,
    ) -> Result<Self, EvidenceError> {
        validate_interval(start_ms, end_ms)?;
        validate_confidence(confidence)?;
        if local_slot_id.as_deref().is_some_and(str::is_empty) {
            return Err(EvidenceError::InvalidSessionSpeaker);
        }
        Ok(Self {
            track_id,
            start_ms,
            end_ms,
            local_slot_id,
            model,
            quality,
            confidence,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SpeakerAttribution {
    Unknown,
    SessionSpeaker(SessionSpeakerAssertion),
    Named(NamedSpeakerAssertion),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpeakerAssertion {
    session_speaker_id: String,
}

impl SessionSpeakerAssertion {
    fn new(session_speaker_id: String) -> Result<Self, EvidenceError> {
        if session_speaker_id.is_empty() {
            return Err(EvidenceError::InvalidSessionSpeaker);
        }
        Ok(Self { session_speaker_id })
    }
}

pub struct ClientSpeakerAttribution;

impl ClientSpeakerAttribution {
    pub fn unknown() -> SpeakerAttribution {
        SpeakerAttribution::Unknown
    }

    pub fn session_speaker(
        session_speaker_id: impl Into<String>,
    ) -> Result<SpeakerAttribution, EvidenceError> {
        Ok(SpeakerAttribution::SessionSpeaker(
            SessionSpeakerAssertion::new(session_speaker_id.into())?,
        ))
    }
}

pub struct ServerSpeakerAttribution;

impl ServerSpeakerAttribution {
    #[allow(clippy::too_many_arguments)]
    pub fn named_from_result(
        identity_id: impl Into<String>,
        profile_revision: impl Into<String>,
        model: ModelRevision,
        confidence: f32,
        purpose_grant_id: impl Into<String>,
        revocation_epoch: u64,
    ) -> Result<SpeakerAttribution, EvidenceError> {
        Ok(SpeakerAttribution::Named(
            NamedSpeakerAssertion::from_server_result(
                identity_id,
                profile_revision,
                model,
                confidence,
                purpose_grant_id,
                revocation_epoch,
            )?,
        ))
    }
}

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedSpeakerAssertion {
    identity_id: String,
    profile_revision: String,
    model: ModelRevision,
    confidence_micros: u32,
    purpose_grant_id: String,
    revocation_epoch: u64,
}

impl std::fmt::Debug for NamedSpeakerAssertion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NamedSpeakerAssertion")
            .field("identity_id", &self.identity_id)
            .field("profile_revision", &self.profile_revision)
            .field("model", &self.model)
            .field("confidence_micros", &self.confidence_micros)
            .field("purpose_grant_id", &self.purpose_grant_id)
            .field("revocation_epoch", &self.revocation_epoch)
            .finish()
    }
}

impl NamedSpeakerAssertion {
    #[allow(clippy::too_many_arguments)]
    fn from_server_result(
        identity_id: impl Into<String>,
        profile_revision: impl Into<String>,
        model: ModelRevision,
        confidence: f32,
        purpose_grant_id: impl Into<String>,
        revocation_epoch: u64,
    ) -> Result<Self, EvidenceError> {
        validate_confidence(Some(confidence))?;
        let confidence_micros = (confidence * 1_000_000.0).round() as u32;
        Self::from_wire(
            identity_id.into(),
            profile_revision.into(),
            model,
            confidence_micros,
            purpose_grant_id.into(),
            revocation_epoch,
        )
    }

    fn from_wire(
        identity_id: String,
        profile_revision: String,
        model: ModelRevision,
        confidence_micros: u32,
        purpose_grant_id: String,
        revocation_epoch: u64,
    ) -> Result<Self, EvidenceError> {
        if identity_id.is_empty() || profile_revision.is_empty() || purpose_grant_id.is_empty() {
            return Err(EvidenceError::MissingNamedAssertionProvenance);
        }
        if confidence_micros > 1_000_000 {
            return Err(EvidenceError::InvalidConfidence);
        }
        Ok(Self {
            identity_id,
            profile_revision,
            model,
            confidence_micros,
            purpose_grant_id,
            revocation_epoch,
        })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerTurn {
    turn_id: String,
    start_ms: u64,
    end_ms: u64,
    attribution: SpeakerAttribution,
    confidence: Option<f32>,
}

impl SpeakerTurn {
    pub fn new(
        turn_id: impl Into<String>,
        start_ms: u64,
        end_ms: u64,
        attribution: SpeakerAttribution,
        confidence: Option<f32>,
    ) -> Result<Self, EvidenceError> {
        let turn_id = turn_id.into();
        if turn_id.is_empty() {
            return Err(EvidenceError::InvalidTurnId);
        }
        validate_interval(start_ms, end_ms)?;
        validate_confidence(confidence)?;
        Ok(Self {
            turn_id,
            start_ms,
            end_ms,
            attribution,
            confidence,
        })
    }

    pub(crate) fn has_named_attribution(&self) -> bool {
        matches!(self.attribution, SpeakerAttribution::Named(_))
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignedWord {
    index: u64,
    text: String,
    start_ms: u64,
    end_ms: u64,
    turn_id: String,
    attribution: SpeakerAttribution,
}

impl AlignedWord {
    pub fn new(
        index: u64,
        text: impl Into<String>,
        start_ms: u64,
        end_ms: u64,
        turn_id: impl Into<String>,
        attribution: SpeakerAttribution,
    ) -> Result<Self, EvidenceError> {
        let text = text.into();
        let turn_id = turn_id.into();
        if text.is_empty() || turn_id.is_empty() {
            return Err(EvidenceError::InvalidAlignedWord);
        }
        validate_interval(start_ms, end_ms)?;
        Ok(Self {
            index,
            text,
            start_ms,
            end_ms,
            turn_id,
            attribution,
        })
    }

    pub(crate) fn has_named_attribution(&self) -> bool {
        matches!(self.attribution, SpeakerAttribution::Named(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceError {
    InvalidInterval,
    InvalidConfidence,
    InvalidSessionSpeaker,
    MissingProvenance,
    MissingNamedAssertionProvenance,
    InvalidTurnId,
    InvalidAlignedWord,
    ClientCannotAssertNamed,
}

impl std::fmt::Display for EvidenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for EvidenceError {}

fn validate_interval(start_ms: u64, end_ms: u64) -> Result<(), EvidenceError> {
    (end_ms > start_ms)
        .then_some(())
        .ok_or(EvidenceError::InvalidInterval)
}

fn validate_confidence(confidence: Option<f32>) -> Result<(), EvidenceError> {
    confidence
        .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        .then_some(())
        .ok_or(EvidenceError::InvalidConfidence)
}

mod wire;

pub(crate) use wire::{ServerAlignedWord, ServerSpeakerTurn};

#[cfg(test)]
mod tests;
