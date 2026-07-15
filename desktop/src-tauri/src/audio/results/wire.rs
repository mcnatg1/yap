use crate::audio::{
    evidence::{ModelRevision, ServerAlignedWord, ServerSpeakerTurn},
    session::SessionId,
};

use super::{
    validation::{validate_named_attribution_authority, validate_wire_revision},
    ResultAuthority, ResultStatus, SpeakerResultRevision, TranscriptResultRevision,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptResultRevisionWire {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    transcript: String,
    aligned_words: Vec<ServerAlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

impl<'de> serde::Deserialize<'de> for TranscriptResultRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = TranscriptResultRevisionWire::deserialize(deserializer)?;
        validate_wire_revision(
            wire.revision,
            &wire.capture_sidecar_sha256,
            wire.previous_result_sha256.as_deref(),
        )
        .map_err(serde::de::Error::custom)?;
        let aligned_words = wire
            .aligned_words
            .into_iter()
            .map(ServerAlignedWord::into_inner)
            .collect::<Vec<_>>();
        validate_named_attribution_authority(wire.authority, &[], &aligned_words)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            session_id: wire.session_id,
            revision: wire.revision,
            authority: wire.authority,
            capture_sidecar_sha256: wire.capture_sidecar_sha256,
            previous_result_sha256: wire.previous_result_sha256,
            status: wire.status,
            transcript: wire.transcript,
            aligned_words,
            model_provenance: wire.model_provenance,
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeakerResultRevisionWire {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    speaker_turns: Vec<ServerSpeakerTurn>,
    aligned_words: Vec<ServerAlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

impl<'de> serde::Deserialize<'de> for SpeakerResultRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SpeakerResultRevisionWire::deserialize(deserializer)?;
        validate_wire_revision(
            wire.revision,
            &wire.capture_sidecar_sha256,
            wire.previous_result_sha256.as_deref(),
        )
        .map_err(serde::de::Error::custom)?;
        let speaker_turns = wire
            .speaker_turns
            .into_iter()
            .map(ServerSpeakerTurn::into_inner)
            .collect::<Vec<_>>();
        let aligned_words = wire
            .aligned_words
            .into_iter()
            .map(ServerAlignedWord::into_inner)
            .collect::<Vec<_>>();
        validate_named_attribution_authority(wire.authority, &speaker_turns, &aligned_words)
            .map_err(serde::de::Error::custom)?;
        Ok(Self {
            session_id: wire.session_id,
            revision: wire.revision,
            authority: wire.authority,
            capture_sidecar_sha256: wire.capture_sidecar_sha256,
            previous_result_sha256: wire.previous_result_sha256,
            status: wire.status,
            speaker_turns,
            aligned_words,
            model_provenance: wire.model_provenance,
        })
    }
}
