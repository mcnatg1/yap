use crate::audio::session::TrackId;

use super::*;

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
