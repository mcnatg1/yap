use reqwest::{Client, Response, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};

use super::config::{self, ConfigError};

const MAX_JOB_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_RESULT_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CREATE_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_JOB_CHUNKS: usize = 4096;
const MAX_CHUNK_BYTES: u64 = 1024 * 1024;
const MAX_JOB_PCM_BYTES: u64 = 16_000 * 2 * 4 * 60 * 60;
const MAX_PRIVATE_RETENTION_DAYS: i64 = 30;

#[derive(Debug)]
pub(crate) enum BatchClientError {
    InvalidOrigin(ConfigError),
    InvalidIdentifier,
    Encode(serde_json::Error),
    Transport(reqwest::Error),
    ResponseTooLarge,
    MalformedResponse,
    InvalidPersistedRequest,
    Api {
        status: StatusCode,
        code: String,
        retryable: bool,
    },
}

impl std::fmt::Display for BatchClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOrigin(error) => write!(formatter, "{error}"),
            Self::InvalidIdentifier => formatter.write_str("Batch request identifier is invalid."),
            Self::Encode(_) => formatter.write_str("Batch request could not be encoded."),
            Self::Transport(error) if error.is_timeout() => {
                formatter.write_str("Batch server request timed out.")
            }
            Self::Transport(_) => formatter.write_str("Batch server request failed."),
            Self::ResponseTooLarge => formatter.write_str("Batch server response is too large."),
            Self::MalformedResponse => {
                formatter.write_str("Batch server returned an incompatible response.")
            }
            Self::InvalidPersistedRequest => {
                formatter.write_str("Prepared batch request is incompatible or corrupt.")
            }
            Self::Api { status, code, .. } => {
                write!(formatter, "{code} (HTTP {})", status.as_u16())
            }
        }
    }
}

impl BatchClientError {
    pub(crate) fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(_) => true,
            Self::Api { retryable, .. } => *retryable,
            Self::InvalidOrigin(_)
            | Self::InvalidIdentifier
            | Self::Encode(_)
            | Self::ResponseTooLarge
            | Self::MalformedResponse
            | Self::InvalidPersistedRequest => false,
        }
    }
}

impl std::error::Error for BatchClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidOrigin(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptureManifestReference {
    pub schema_version: u16,
    pub session_id: String,
    pub sha256: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServerReplayKey {
    pub schema_version: u16,
    pub session_id: String,
    pub track_id: String,
    pub sequence_start: u64,
    pub sequence_end: u64,
}

impl ServerReplayKey {
    fn idempotency_key(&self) -> String {
        format!(
            "{}/{}/{}/{}/{}",
            self.schema_version,
            self.session_id,
            self.track_id,
            self.sequence_start,
            self.sequence_end
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ContentIdentity {
    pub sha256: String,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaptureChunkReference {
    pub replay_key: ServerReplayKey,
    pub content_identity: ContentIdentity,
    pub audio_codec: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub start_ms: u64,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UploadTrack {
    pub track_id: String,
    pub source: serde_json::Value,
    pub device_id: Option<String>,
    pub original_sample_rate_hz: u32,
    pub original_channels: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateRecordingJobRequest {
    pub display_name: String,
    pub metadata: crate::audio::session::SessionMetadata,
    pub tracks: Vec<UploadTrack>,
    pub route: String,
    pub capture_manifest: CaptureManifestReference,
    pub chunks: Vec<CaptureChunkReference>,
}

impl CreateRecordingJobRequest {
    pub(crate) fn decode_persisted(encoded: &str) -> Result<Self, BatchClientError> {
        if encoded.len() < 2 || encoded.len() > MAX_CREATE_REQUEST_BYTES {
            return Err(BatchClientError::InvalidPersistedRequest);
        }
        let raw: serde_json::Value =
            serde_json::from_str(encoded).map_err(|_| BatchClientError::InvalidPersistedRequest)?;
        let request: Self = serde_json::from_value(raw.clone())
            .map_err(|_| BatchClientError::InvalidPersistedRequest)?;
        let canonical = serde_json::to_value(&request)
            .map_err(|_| BatchClientError::InvalidPersistedRequest)?;
        if raw != canonical || !request.is_valid_current_slice() {
            return Err(BatchClientError::InvalidPersistedRequest);
        }
        Ok(request)
    }

    pub(crate) fn create_idempotency_key(&self) -> Result<String, BatchClientError> {
        let encoded = serde_json::to_vec(self).map_err(BatchClientError::Encode)?;
        let digest = Sha256::digest(encoded);
        let hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(format!("create-{hex}"))
    }

    fn is_valid_current_slice(&self) -> bool {
        use crate::audio::session::{SessionMode, SessionOrigin};

        if self.display_name.is_empty()
            || self.display_name.len() > 256
            || self.route != "server_batch"
            || self.metadata.mode != SessionMode::Meeting
            || self.metadata.origin != SessionOrigin::ImportedFile
            || self.metadata.preferred_languages_bcp47.is_empty()
            || !valid_private_retention(&self.metadata)
            || self.tracks.len() != 1
            || self.chunks.is_empty()
            || self.chunks.len() > MAX_JOB_CHUNKS
        {
            return false;
        }
        let session_id = self.metadata.session_id.as_str();
        if self.capture_manifest.schema_version != 1
            || self.capture_manifest.session_id != session_id
            || !valid_sha256(&self.capture_manifest.sha256)
            || self.capture_manifest.byte_length == 0
        {
            return false;
        }
        let track = &self.tracks[0];
        if !valid_path_segment(&track.track_id)
            || track.device_id.is_some()
            || track.original_sample_rate_hz != 16_000
            || track.original_channels != 1
            || track.source != serde_json::json!({"kind": "imported", "provenance": "unknown"})
        {
            return false;
        }

        let mut expected_sequence_start = 0_u64;
        let mut expected_start_ms = 0_u64;
        let mut total_pcm_bytes = 0_u64;
        for chunk in &self.chunks {
            let replay = &chunk.replay_key;
            let content = &chunk.content_identity;
            if replay.schema_version != 1
                || replay.session_id != session_id
                || replay.track_id != track.track_id
                || replay.sequence_start != expected_sequence_start
                || replay.sequence_end < replay.sequence_start
                || !valid_sha256(&content.sha256)
                || !(2..=MAX_CHUNK_BYTES).contains(&content.byte_length)
                || content.byte_length % 2 != 0
                || chunk.audio_codec != "pcm_s16le"
                || chunk.sample_rate_hz != 16_000
                || chunk.channels != 1
                || chunk.start_ms != expected_start_ms
                || chunk.duration_ms == 0
            {
                return false;
            }
            let sample_count = replay
                .sequence_end
                .checked_sub(replay.sequence_start)
                .and_then(|count| count.checked_add(1));
            if sample_count != Some(content.byte_length / 2)
                || u128::from(content.byte_length) * 1000
                    != u128::from(chunk.duration_ms) * 16_000 * 2
            {
                return false;
            }
            let Some(next_total_pcm_bytes) = total_pcm_bytes.checked_add(content.byte_length)
            else {
                return false;
            };
            if next_total_pcm_bytes > MAX_JOB_PCM_BYTES {
                return false;
            }
            let Some(next_sequence) = replay.sequence_end.checked_add(1) else {
                return false;
            };
            let Some(next_start_ms) = expected_start_ms.checked_add(u64::from(chunk.duration_ms))
            else {
                return false;
            };
            total_pcm_bytes = next_total_pcm_bytes;
            expected_sequence_start = next_sequence;
            expected_start_ms = next_start_ms;
        }
        true
    }
}

fn valid_private_retention(metadata: &crate::audio::session::SessionMetadata) -> bool {
    let Some(retention) = metadata.retention_expires_at_utc.as_deref() else {
        return false;
    };
    if !metadata.started_at_utc.ends_with('Z') || !retention.ends_with('Z') {
        return false;
    }
    let Ok(started_at) = OffsetDateTime::parse(&metadata.started_at_utc, &Rfc3339) else {
        return false;
    };
    let Ok(retention_at) = OffsetDateTime::parse(retention, &Rfc3339) else {
        return false;
    };
    retention_at > started_at
        && retention_at - started_at <= TimeDuration::days(MAX_PRIVATE_RETENTION_DAYS)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CommitRecordingJobRequest {
    pub capture_manifest: CaptureManifestReference,
    pub chunk_count: usize,
}

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

#[derive(Clone)]
pub(crate) struct BatchApiClient {
    client: Client,
    base_url: Url,
    base_url_identity: String,
}

impl BatchApiClient {
    pub(crate) fn new(client: Client, base_url: &str) -> Result<Self, BatchClientError> {
        let normalized = validate_development_batch_base_url(base_url)
            .map_err(BatchClientError::InvalidOrigin)?;
        let base_url = Url::parse(&normalized).map_err(|_| BatchClientError::MalformedResponse)?;
        Ok(Self {
            client,
            base_url,
            base_url_identity: normalized,
        })
    }

    pub(crate) fn base_url_identity(&self) -> &str {
        &self.base_url_identity
    }

    pub(crate) async fn create(
        &self,
        idempotency_key: &str,
        request: &CreateRecordingJobRequest,
    ) -> Result<RecordingJob, BatchClientError> {
        if !valid_path_segment(idempotency_key) {
            return Err(BatchClientError::InvalidIdentifier);
        }
        let response = self
            .client
            .post(self.endpoint(&["jobs"])?)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("Idempotency-Key", idempotency_key)
            .body(serde_json::to_vec(request).map_err(BatchClientError::Encode)?)
            .send()
            .await
            .map_err(BatchClientError::Transport)?;
        decode_response(response, &[StatusCode::ACCEPTED], MAX_JOB_RESPONSE_BYTES).await
    }

    pub(crate) async fn upload_chunk(
        &self,
        job_id: &str,
        chunk: &CaptureChunkReference,
        body: Vec<u8>,
    ) -> Result<ChunkUploadReceipt, BatchClientError> {
        if body.len() as u64 != chunk.content_identity.byte_length {
            return Err(BatchClientError::MalformedResponse);
        }
        let range = format!(
            "{}-{}",
            chunk.replay_key.sequence_start, chunk.replay_key.sequence_end
        );
        let response = self
            .client
            .put(self.endpoint(&["jobs", job_id, "chunks", &chunk.replay_key.track_id, &range])?)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .header("Idempotency-Key", chunk.replay_key.idempotency_key())
            .header("X-Yap-Content-SHA256", &chunk.content_identity.sha256)
            .header("X-Yap-Audio-Codec", &chunk.audio_codec)
            .header("X-Yap-Sample-Rate-Hz", chunk.sample_rate_hz)
            .header("X-Yap-Channels", chunk.channels)
            .body(body)
            .send()
            .await
            .map_err(BatchClientError::Transport)?;
        decode_response(
            response,
            &[StatusCode::OK, StatusCode::CREATED],
            MAX_JOB_RESPONSE_BYTES,
        )
        .await
    }

    pub(crate) async fn commit(
        &self,
        job_id: &str,
        request: &CommitRecordingJobRequest,
    ) -> Result<RecordingJob, BatchClientError> {
        let response = self
            .client
            .post(self.endpoint(&["jobs", job_id, "commit"])?)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(request).map_err(BatchClientError::Encode)?)
            .send()
            .await
            .map_err(BatchClientError::Transport)?;
        decode_response(response, &[StatusCode::ACCEPTED], MAX_JOB_RESPONSE_BYTES).await
    }

    pub(crate) async fn status(&self, job_id: &str) -> Result<RecordingJob, BatchClientError> {
        let response = self
            .client
            .get(self.endpoint(&["jobs", job_id])?)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(BatchClientError::Transport)?;
        decode_response(response, &[StatusCode::OK], MAX_JOB_RESPONSE_BYTES).await
    }

    pub(crate) async fn result(
        &self,
        job_id: &str,
    ) -> Result<TranscriptResultRevision, BatchClientError> {
        let response = self
            .client
            .get(self.endpoint(&["jobs", job_id, "result"])?)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(BatchClientError::Transport)?;
        decode_response(response, &[StatusCode::OK], MAX_RESULT_RESPONSE_BYTES).await
    }

    pub(crate) async fn cancel(&self, job_id: &str) -> Result<RecordingJob, BatchClientError> {
        let response = self
            .client
            .delete(self.endpoint(&["jobs", job_id])?)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(BatchClientError::Transport)?;
        decode_response(response, &[StatusCode::ACCEPTED], MAX_JOB_RESPONSE_BYTES).await
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, BatchClientError> {
        if segments.iter().any(|segment| !valid_path_segment(segment)) {
            return Err(BatchClientError::InvalidIdentifier);
        }
        let mut url = self.base_url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| BatchClientError::InvalidIdentifier)?;
            path.clear().push("v1");
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }
}

fn valid_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

async fn decode_response<T: DeserializeOwned>(
    response: Response,
    successes: &[StatusCode],
    maximum_bytes: usize,
) -> Result<T, BatchClientError> {
    let status = response.status();
    let body = read_bounded(response, maximum_bytes).await?;
    if !successes.contains(&status) {
        let error: ApiError =
            serde_json::from_slice(&body).map_err(|_| BatchClientError::MalformedResponse)?;
        if !error.is_valid() {
            return Err(BatchClientError::MalformedResponse);
        }
        return Err(BatchClientError::Api {
            status,
            code: error.code,
            retryable: error.retryable,
        });
    }
    serde_json::from_slice(&body).map_err(|_| BatchClientError::MalformedResponse)
}

async fn read_bounded(
    mut response: Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, BatchClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(BatchClientError::ResponseTooLarge);
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(maximum_bytes as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(BatchClientError::Transport)?
    {
        if body.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err(BatchClientError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(crate) fn validate_development_batch_base_url(raw: &str) -> Result<String, ConfigError> {
    let normalized = config::validate_base_url(raw, false)?;
    let url =
        Url::parse(&normalized).map_err(|_| ConfigError::Invalid("Enter a valid server URL."))?;
    let host = url
        .host_str()
        .ok_or(ConfigError::Invalid("Server URL must include a host."))?;
    let is_loopback = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback());
    if !is_loopback {
        return Err(ConfigError::Invalid(
            "Remote audio requires a loopback SSH tunnel until authenticated server transport ships.",
        ));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use crate::audio::session::{
        SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode,
    };

    use super::{
        validate_development_batch_base_url, ApiError, BatchClientError, CaptureChunkReference,
        CaptureManifestReference, ContentIdentity, CreateRecordingJobRequest, ServerReplayKey,
        UploadTrack,
    };

    #[test]
    fn unauthenticated_audio_transport_accepts_only_loopback_tunnel_origins() {
        assert_eq!(
            validate_development_batch_base_url("http://127.0.0.1:18765").unwrap(),
            "http://127.0.0.1:18765"
        );
        assert_eq!(
            validate_development_batch_base_url("http://[::1]:18765/v1").unwrap(),
            "http://[::1]:18765"
        );
        assert!(validate_development_batch_base_url("http://localhost:18765").is_err());
        assert!(validate_development_batch_base_url("http://192.168.50.1:18765").is_err());
        assert!(validate_development_batch_base_url("https://yap.internal").is_err());
    }

    #[test]
    fn persisted_create_request_round_trips_strictly_before_resume() {
        let started = UNIX_EPOCH + Duration::from_secs(1_720_000_000);
        let session_id = "s-persisted-request";
        let request = CreateRecordingJobRequest {
            display_name: "interview.wav".into(),
            metadata: SessionMetadata::new(
                SessionId::new(session_id).unwrap(),
                SessionMode::Meeting,
                SessionOrigin::ImportedFile,
                TriggerMode::Toggle,
                started,
                None,
                Some("en-US".into()),
                None,
                vec!["en-US".into()],
                Some(started + Duration::from_secs(3600)),
            )
            .unwrap(),
            tracks: vec![UploadTrack {
                track_id: "track-1".into(),
                source: serde_json::json!({"kind": "imported", "provenance": "unknown"}),
                device_id: None,
                original_sample_rate_hz: 16_000,
                original_channels: 1,
            }],
            route: "server_batch".into(),
            capture_manifest: CaptureManifestReference {
                schema_version: 1,
                session_id: session_id.into(),
                sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                byte_length: 200,
            },
            chunks: vec![CaptureChunkReference {
                replay_key: ServerReplayKey {
                    schema_version: 1,
                    session_id: session_id.into(),
                    track_id: "track-1".into(),
                    sequence_start: 0,
                    sequence_end: 159,
                },
                content_identity: ContentIdentity {
                    sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                        .into(),
                    byte_length: 320,
                },
                audio_codec: "pcm_s16le".into(),
                sample_rate_hz: 16_000,
                channels: 1,
                start_ms: 0,
                duration_ms: 10,
            }],
        };
        let encoded = serde_json::to_string(&request).unwrap();
        let original_key = request.create_idempotency_key().unwrap();

        assert_eq!(
            CreateRecordingJobRequest::decode_persisted(&encoded).unwrap(),
            request
        );
        assert_eq!(request.create_idempotency_key().unwrap(), original_key);
        let mut new_attempt = request.clone();
        new_attempt.display_name = "a distinct immutable request".into();
        assert_ne!(new_attempt.create_idempotency_key().unwrap(), original_key);
        let with_unknown = encoded.replacen('{', r#"{"unexpected":true,"#, 1);
        assert!(CreateRecordingJobRequest::decode_persisted(&with_unknown).is_err());
        let mut missing_retention = request.clone();
        missing_retention.metadata.retention_expires_at_utc = None;
        assert!(CreateRecordingJobRequest::decode_persisted(
            &serde_json::to_string(&missing_retention).unwrap()
        )
        .is_err());
        let mut unbounded_retention = request.clone();
        unbounded_retention.metadata.retention_expires_at_utc = Some("2126-07-14T21:00:00Z".into());
        assert!(CreateRecordingJobRequest::decode_persisted(
            &serde_json::to_string(&unbounded_retention).unwrap()
        )
        .is_err());

        const FOUR_HOURS_PCM_BYTES: u64 = 16_000 * 2 * 4 * 60 * 60;
        let mut oversized = request;
        let chunk_bytes = 960_000_u64;
        let chunk_frames = chunk_bytes / 2;
        let chunk_duration_ms = 30_000_u32;
        oversized.chunks = (0..=(FOUR_HOURS_PCM_BYTES / chunk_bytes))
            .map(|index| {
                let sequence_start = index * chunk_frames;
                CaptureChunkReference {
                    replay_key: ServerReplayKey {
                        schema_version: 1,
                        session_id: session_id.into(),
                        track_id: "track-1".into(),
                        sequence_start,
                        sequence_end: sequence_start + chunk_frames - 1,
                    },
                    content_identity: ContentIdentity {
                        sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                            .into(),
                        byte_length: chunk_bytes,
                    },
                    audio_codec: "pcm_s16le".into(),
                    sample_rate_hz: 16_000,
                    channels: 1,
                    start_ms: index * u64::from(chunk_duration_ms),
                    duration_ms: chunk_duration_ms,
                }
            })
            .collect();
        assert!(CreateRecordingJobRequest::decode_persisted(
            &serde_json::to_string(&oversized).unwrap()
        )
        .is_err());
    }

    #[test]
    fn server_retryability_is_preserved_as_typed_transport_state() {
        let retryable = BatchClientError::Api {
            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
            code: "POOL_BUSY".into(),
            retryable: true,
        };
        let terminal = BatchClientError::Api {
            status: reqwest::StatusCode::CONFLICT,
            code: "MANIFEST_CONFLICT".into(),
            retryable: false,
        };

        assert!(retryable.is_retryable());
        assert!(!terminal.is_retryable());
        assert!(!BatchClientError::MalformedResponse.is_retryable());
    }

    #[test]
    fn server_error_fields_are_bounded_before_logging_or_retry_decisions() {
        let valid = ApiError {
            code: "POOL_BUSY".into(),
            message: "Try again.".into(),
            retryable: true,
            request_id: "job-abc123".into(),
        };
        assert!(valid.is_valid());

        let mut injected_line = valid.clone();
        injected_line.message = "Try again.\nforged log entry".into();
        assert!(!injected_line.is_valid());

        let mut invalid_request_id = valid;
        invalid_request_id.request_id = "../../outside".into();
        assert!(!invalid_request_id.is_valid());
    }
}
