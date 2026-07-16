use super::{
    artifact_io::{
        next_staging_nonce, sha256_bytes, sha256_reader, validate_identifier, write_new_synced,
        StagingDirectory,
    },
    spool::prepare_spool_root,
    wav::inspect_pcm_wav,
};
use crate::{
    audio::session::{
        OwnerNamespace, SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode,
    },
    jobs::{NewJobChunk, NewPreparedRemoteJob},
    server_connector::batch::{
        CaptureChunkReference, CaptureManifestReference, ContentIdentity,
        CreateRecordingJobRequest, ServerReplayKey, UploadTrack,
    },
};
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const CHUNK_PCM_BYTES: usize = 960_000;
const PCM_BYTES_PER_MILLISECOND: usize = 32;
const RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;

pub(in crate::jobs) struct PreparedRemoteChunk {
    pub(in crate::jobs) reference: CaptureChunkReference,
    pub(in crate::jobs) artifact_path: PathBuf,
}

pub(in crate::jobs) struct PreparedRemoteJob {
    pub(in crate::jobs) request: CreateRecordingJobRequest,
    pub(in crate::jobs) chunks: Vec<PreparedRemoteChunk>,
    pub(in crate::jobs) capture_manifest_path: PathBuf,
    pub(in crate::jobs) owner_namespace: String,
}

impl PreparedRemoteJob {
    pub(in crate::jobs) fn into_ledger_state(self) -> Result<NewPreparedRemoteJob, String> {
        let create_request_json = serde_json::to_string(&self.request)
            .map_err(|error| format!("failed to encode prepared server request: {error}"))?;
        let capture_manifest_sha256 = self.request.capture_manifest.sha256.clone();
        let chunks = self
            .chunks
            .into_iter()
            .map(|chunk| NewJobChunk {
                owner_namespace: self.owner_namespace.clone(),
                session_id: chunk.reference.replay_key.session_id,
                track_id: chunk.reference.replay_key.track_id,
                sequence_start: chunk.reference.replay_key.sequence_start,
                sequence_end: chunk.reference.replay_key.sequence_end,
                content_sha256: chunk.reference.content_identity.sha256,
                content_byte_length: chunk.reference.content_identity.byte_length,
                artifact_path: chunk.artifact_path,
                upload_offset: 0,
                acknowledged_object_id: None,
                acknowledged_at_ms: None,
            })
            .collect();
        Ok(NewPreparedRemoteJob {
            create_request_json,
            capture_manifest_path: self.capture_manifest_path,
            capture_manifest_sha256,
            chunks,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportedCaptureManifest<'a> {
    schema_version: u16,
    session_id: &'a str,
    source: ImportedSourceIdentity<'a>,
    preprocessing: ImportedPreprocessing,
    chunks: &'a [CaptureChunkReference],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportedSourceIdentity<'a> {
    display_name: &'a str,
    sha256: String,
    byte_length: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportedPreprocessing {
    audio_codec: &'static str,
    sample_rate_hz: u32,
    channels: u16,
    padded_final_millisecond: bool,
}

pub(in crate::jobs) fn prepare_imported_pcm_wav(
    job_id: &str,
    display_name: &str,
    source: &mut File,
    spool_root: &Path,
    owner_namespace: &OwnerNamespace,
    started_at: SystemTime,
) -> Result<PreparedRemoteJob, String> {
    validate_identifier(job_id, 128, "job ID")?;
    if display_name.is_empty() || display_name.len() > 256 {
        return Err("display name is outside the server contract".into());
    }
    let wav = inspect_pcm_wav(source)?;
    let source_sha256 = sha256_reader(source, wav.source_bytes)?;
    source
        .seek(SeekFrom::Start(wav.data_offset))
        .map_err(|error| format!("failed to seek imported WAV data: {error}"))?;

    prepare_spool_root(spool_root)?;
    let destination = spool_root.join(job_id);
    if destination.exists() {
        return Err("a prepared spool already exists for this recording job".into());
    }
    let nonce = next_staging_nonce();
    let staging_path = spool_root.join(format!(".{job_id}-{}-{nonce}.part", std::process::id()));
    let mut staging = StagingDirectory::create(staging_path)?;

    let session_id = format!("s-{}", job_id.strip_prefix("job-").unwrap_or(job_id));
    let session = SessionId::new(session_id.clone())?;
    let mut metadata = SessionMetadata::new(
        session,
        SessionMode::Meeting,
        SessionOrigin::ImportedFile,
        TriggerMode::Toggle,
        started_at,
        None,
        Some("en-US".into()),
        None,
        vec!["en-US".into()],
        Some(started_at + Duration::from_secs(RETENTION_SECONDS)),
    )?;
    metadata.privacy_policy_version = "development-only".into();

    let track_id = "track-1".to_string();
    let mut remaining = wav.data_bytes;
    let mut sequence_start = 0_u64;
    let mut start_ms = 0_u64;
    let mut references = Vec::new();
    let mut staged_names = Vec::new();
    let mut padded_final_millisecond = false;
    while remaining > 0 {
        let read_length = remaining.min(CHUNK_PCM_BYTES as u64) as usize;
        let mut body = vec![0_u8; read_length];
        source
            .read_exact(&mut body)
            .map_err(|error| format!("failed to read imported WAV audio: {error}"))?;
        remaining -= read_length as u64;
        if remaining == 0 {
            let padded_length =
                body.len().div_ceil(PCM_BYTES_PER_MILLISECOND) * PCM_BYTES_PER_MILLISECOND;
            if padded_length != body.len() {
                body.resize(padded_length, 0);
                padded_final_millisecond = true;
            }
        }
        let sample_count = u64::try_from(body.len() / 2)
            .map_err(|_| "imported WAV sample count is out of range")?;
        let sequence_end = sequence_start
            .checked_add(sample_count)
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| "imported WAV sequence range overflowed".to_string())?;
        let duration_ms = u32::try_from(body.len() / PCM_BYTES_PER_MILLISECOND)
            .map_err(|_| "imported WAV chunk duration is out of range")?;
        let filename = format!("{track_id}-{sequence_start}-{sequence_end}.pcm");
        write_new_synced(&staging.path.join(&filename), &body)?;
        let reference = CaptureChunkReference {
            replay_key: ServerReplayKey {
                schema_version: 1,
                session_id: session_id.clone(),
                track_id: track_id.clone(),
                sequence_start,
                sequence_end,
            },
            content_identity: ContentIdentity {
                sha256: sha256_bytes(&body),
                byte_length: body.len() as u64,
            },
            audio_codec: "pcm_s16le".into(),
            sample_rate_hz: 16_000,
            channels: 1,
            start_ms,
            duration_ms,
        };
        start_ms = start_ms
            .checked_add(u64::from(duration_ms))
            .ok_or_else(|| "imported WAV timeline overflowed".to_string())?;
        sequence_start = sequence_end
            .checked_add(1)
            .ok_or_else(|| "imported WAV sequence overflowed".to_string())?;
        references.push(reference);
        staged_names.push(filename);
    }
    if references.is_empty() {
        return Err("imported WAV contains no audio samples".into());
    }

    let manifest = ImportedCaptureManifest {
        schema_version: 1,
        session_id: &session_id,
        source: ImportedSourceIdentity {
            display_name,
            sha256: source_sha256,
            byte_length: wav.source_bytes,
        },
        preprocessing: ImportedPreprocessing {
            audio_codec: "pcm_s16le",
            sample_rate_hz: 16_000,
            channels: 1,
            padded_final_millisecond,
        },
        chunks: &references,
    };
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| format!("failed to encode capture manifest: {error}"))?;
    let manifest_name = "capture-manifest.json";
    write_new_synced(&staging.path.join(manifest_name), &manifest_bytes)?;
    staging.publish(&destination)?;

    let capture_manifest = CaptureManifestReference {
        schema_version: 1,
        session_id,
        sha256: sha256_bytes(&manifest_bytes),
        byte_length: manifest_bytes.len() as u64,
    };
    let chunks = references
        .iter()
        .cloned()
        .zip(staged_names)
        .map(|(reference, filename)| PreparedRemoteChunk {
            reference,
            artifact_path: destination.join(filename),
        })
        .collect();
    Ok(PreparedRemoteJob {
        request: CreateRecordingJobRequest {
            display_name: display_name.into(),
            metadata,
            tracks: vec![UploadTrack {
                track_id,
                source: serde_json::json!({
                    "kind": "imported",
                    "provenance": "unknown"
                }),
                device_id: None,
                original_sample_rate_hz: 16_000,
                original_channels: 1,
            }],
            route: "server_batch".into(),
            capture_manifest,
            chunks: references,
        },
        chunks,
        capture_manifest_path: destination.join(manifest_name),
        owner_namespace: owner_namespace.as_str().into(),
    })
}
