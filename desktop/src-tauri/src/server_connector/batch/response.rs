use serde::{Deserialize, Serialize};

use super::validation::valid_path_segment;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecordingJob {
    pub job_id: String,
    pub session_id: String,
    pub display_name: String,
    pub session_mode: String,
    pub session_origin: String,
    pub status: String,
    pub route: Option<String>,
    pub capture_manifest: CaptureManifestReferenceWire,
    pub progress_percent: Option<f64>,
    pub progress_message: Option<String>,
    pub error: Option<ApiError>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptureManifestReferenceWire {
    pub schema_version: u16,
    pub session_id: String,
    pub sha256: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApiError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub request_id: String,
}

impl ApiError {
    pub(crate) fn is_valid(&self) -> bool {
        !self.code.is_empty()
            && self.code.len() <= 64
            && self
                .code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            && !self.message.is_empty()
            && self.message.len() <= 512
            && !self.message.chars().any(char::is_control)
            && valid_path_segment(&self.request_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ChunkUploadReceipt {
    pub replay_key: ServerReplayKeyWire,
    pub content_identity: ContentIdentityWire,
    pub disposition: String,
    pub accepted_at_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServerReplayKeyWire {
    pub schema_version: u16,
    pub session_id: String,
    pub track_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ContentIdentityWire {
    pub sha256: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TranscriptResultRevision {
    pub session_id: String,
    pub revision: u64,
    pub authority: String,
    pub created_at_utc: String,
    pub capture_manifest_sha256: String,
    pub previous_result_sha256: Option<String>,
    pub status: String,
    pub language: Option<LanguageDecision>,
    pub transcript: String,
    pub aligned_words: Vec<serde_json::Value>,
    pub model_provenance: Vec<ModelRevision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LanguageDecision {
    pub language_bcp47: String,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelRevision {
    pub model_id: String,
    pub revision: String,
    pub calibration_revision: String,
}
