use reqwest::{Client, Response, StatusCode, Url};
use serde::de::DeserializeOwned;

use super::{
    super::config::{self, ConfigError},
    validation::valid_path_segment,
    ApiError, BatchClientError, CaptureChunkReference, ChunkUploadReceipt,
    CommitRecordingJobRequest, CreateRecordingJobRequest, RecordingJob, TranscriptResultRevision,
};

const MAX_JOB_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_RESULT_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

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
