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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelRevisionWire {
    model_id: String,
    revision: String,
    calibration_revision: String,
}

impl TryFrom<ModelRevisionWire> for ModelRevision {
    type Error = EvidenceError;

    fn try_from(wire: ModelRevisionWire) -> Result<Self, Self::Error> {
        Self::new(wire.model_id, wire.revision, wire.calibration_revision)
    }
}

impl<'de> serde::Deserialize<'de> for ModelRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        ModelRevisionWire::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerEvidenceWire {
    track_id: TrackId,
    start_ms: u64,
    end_ms: u64,
    local_slot_id: Option<String>,
    model: ModelRevision,
    quality: EvidenceQuality,
    confidence: Option<f32>,
}

impl<'de> serde::Deserialize<'de> for SpeakerEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SpeakerEvidenceWire::deserialize(deserializer)?;
        Self::new(
            wire.track_id,
            wire.start_ms,
            wire.end_ms,
            wire.local_slot_id,
            wire.model,
            wire.quality,
            wire.confidence,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamedSpeakerAssertionWire {
    identity_id: String,
    profile_revision: String,
    model: ModelRevision,
    confidence_micros: u32,
    purpose_grant_id: String,
    revocation_epoch: u64,
}

impl TryFrom<NamedSpeakerAssertionWire> for NamedSpeakerAssertion {
    type Error = EvidenceError;

    fn try_from(wire: NamedSpeakerAssertionWire) -> Result<Self, Self::Error> {
        Self::from_wire(
            wire.identity_id,
            wire.profile_revision,
            wire.model,
            wire.confidence_micros,
            wire.purpose_grant_id,
            wire.revocation_epoch,
        )
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum SpeakerAttributionWire {
    Unknown,
    SessionSpeaker(SessionSpeakerAssertionWire),
    Named(NamedSpeakerAssertionWire),
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionSpeakerAssertionWire {
    session_speaker_id: String,
}

impl SpeakerAttributionWire {
    fn into_client(self) -> Result<SpeakerAttribution, EvidenceError> {
        match self {
            Self::Unknown => Ok(ClientSpeakerAttribution::unknown()),
            Self::SessionSpeaker(assertion) => {
                ClientSpeakerAttribution::session_speaker(assertion.session_speaker_id)
            }
            Self::Named(_) => Err(EvidenceError::ClientCannotAssertNamed),
        }
    }

    fn into_server(self) -> Result<SpeakerAttribution, EvidenceError> {
        match self {
            Self::Unknown => Ok(ClientSpeakerAttribution::unknown()),
            Self::SessionSpeaker(assertion) => {
                ClientSpeakerAttribution::session_speaker(assertion.session_speaker_id)
            }
            Self::Named(assertion) => Ok(SpeakerAttribution::Named(assertion.try_into()?)),
        }
    }
}

impl<'de> serde::Deserialize<'de> for SpeakerAttribution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        SpeakerAttributionWire::deserialize(deserializer)?
            .into_client()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerTurnWire {
    turn_id: String,
    start_ms: u64,
    end_ms: u64,
    attribution: SpeakerAttribution,
    confidence: Option<f32>,
}

impl<'de> serde::Deserialize<'de> for SpeakerTurn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SpeakerTurnWire::deserialize(deserializer)?;
        Self::new(
            wire.turn_id,
            wire.start_ms,
            wire.end_ms,
            wire.attribution,
            wire.confidence,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlignedWordWire {
    index: u64,
    text: String,
    start_ms: u64,
    end_ms: u64,
    turn_id: String,
    attribution: SpeakerAttribution,
}

impl<'de> serde::Deserialize<'de> for AlignedWord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AlignedWordWire::deserialize(deserializer)?;
        Self::new(
            wire.index,
            wire.text,
            wire.start_ms,
            wire.end_ms,
            wire.turn_id,
            wire.attribution,
        )
        .map_err(serde::de::Error::custom)
    }
}

pub(crate) struct ServerSpeakerTurn(SpeakerTurn);

impl ServerSpeakerTurn {
    pub(crate) fn into_inner(self) -> SpeakerTurn {
        self.0
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerSpeakerTurnWire {
    turn_id: String,
    start_ms: u64,
    end_ms: u64,
    attribution: SpeakerAttributionWire,
    confidence: Option<f32>,
}

impl<'de> serde::Deserialize<'de> for ServerSpeakerTurn {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ServerSpeakerTurnWire::deserialize(deserializer)?;
        let attribution = wire
            .attribution
            .into_server()
            .map_err(serde::de::Error::custom)?;
        SpeakerTurn::new(
            wire.turn_id,
            wire.start_ms,
            wire.end_ms,
            attribution,
            wire.confidence,
        )
        .map(Self)
        .map_err(serde::de::Error::custom)
    }
}

pub(crate) struct ServerAlignedWord(AlignedWord);

impl ServerAlignedWord {
    pub(crate) fn into_inner(self) -> AlignedWord {
        self.0
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerAlignedWordWire {
    index: u64,
    text: String,
    start_ms: u64,
    end_ms: u64,
    turn_id: String,
    attribution: SpeakerAttributionWire,
}

impl<'de> serde::Deserialize<'de> for ServerAlignedWord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ServerAlignedWordWire::deserialize(deserializer)?;
        let attribution = wire
            .attribution
            .into_server()
            .map_err(serde::de::Error::custom)?;
        AlignedWord::new(
            wire.index,
            wire.text,
            wire.start_ms,
            wire.end_ms,
            wire.turn_id,
            attribution,
        )
        .map(Self)
        .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientSpeakerAttribution, SpeakerAttribution};

    #[test]
    fn client_evidence_builder_can_emit_only_unknown_or_session_speaker() {
        assert_eq!(
            ClientSpeakerAttribution::unknown(),
            SpeakerAttribution::Unknown
        );
        let session_speaker = ClientSpeakerAttribution::session_speaker("speaker-1").unwrap();
        assert!(matches!(
            session_speaker,
            SpeakerAttribution::SessionSpeaker(_)
        ));
        assert!(ClientSpeakerAttribution::session_speaker("").is_err());
    }

    #[test]
    fn evidence_json_cannot_bypass_interval_confidence_or_named_provenance_validation() {
        let invalid_evidence = serde_json::json!({
            "trackId": "mic-1",
            "startMs": 20,
            "endMs": 20,
            "localSlotId": "slot-1",
            "model": {
                "modelId": "speaker-model",
                "revision": "r1",
                "calibrationRevision": "calibration-r1"
            },
            "quality": "clean",
            "confidence": 1.2
        });
        assert!(serde_json::from_value::<super::SpeakerEvidence>(invalid_evidence).is_err());

        let invalid_named = serde_json::json!({
            "named": {
                "identityId": "",
                "profileRevision": "profile-r1",
                "model": {
                    "modelId": "speaker-model",
                    "revision": "r1",
                    "calibrationRevision": "calibration-r1"
                },
                "confidenceMicros": 1_100_000,
                "purposeGrantId": "grant-1",
                "revocationEpoch": 1
            }
        });
        assert!(serde_json::from_value::<super::SpeakerAttribution>(invalid_named).is_err());
    }

    #[test]
    fn client_attribution_json_cannot_mint_a_named_speaker() {
        let named = serde_json::json!({
            "named": {
                "identityId": "identity-1",
                "profileRevision": "profile-r1",
                "model": {
                    "modelId": "speaker-model",
                    "revision": "r1",
                    "calibrationRevision": "calibration-r1"
                },
                "confidenceMicros": 900_000,
                "purposeGrantId": "grant-1",
                "revocationEpoch": 1
            }
        });

        assert!(serde_json::from_value::<super::SpeakerAttribution>(named).is_err());
    }
}
