mod client;
mod error;
mod request;
mod response;
mod validation;

pub(crate) use client::{validate_development_batch_base_url, BatchApiClient};
pub(crate) use error::BatchClientError;
pub(crate) use request::{
    CaptureChunkReference, CaptureManifestReference, CommitRecordingJobRequest, ContentIdentity,
    CreateRecordingJobRequest, ServerReplayKey, UploadTrack,
};
pub(crate) use response::{ApiError, ChunkUploadReceipt, RecordingJob, TranscriptResultRevision};
#[cfg(test)]
pub(crate) use response::{LanguageDecision, ModelRevision};

#[cfg(test)]
mod tests;
