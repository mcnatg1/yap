use crate::audio::{
    evidence::{AlignedWord, ModelRevision, SpeakerTurn},
    session::SessionId,
};

mod revision;
mod validation;
mod wire;

pub use validation::ResultRevisionError;

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

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptResultRevision {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    transcript: String,
    aligned_words: Vec<AlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerResultRevision {
    session_id: SessionId,
    revision: u64,
    authority: ResultAuthority,
    capture_sidecar_sha256: String,
    previous_result_sha256: Option<String>,
    status: ResultStatus,
    speaker_turns: Vec<SpeakerTurn>,
    aligned_words: Vec<AlignedWord>,
    model_provenance: Vec<ModelRevision>,
}

#[cfg(test)]
mod tests;
