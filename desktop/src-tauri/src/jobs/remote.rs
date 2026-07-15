use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime},
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::{
    audio::session::{
        OwnerNamespace, SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode,
    },
    jobs::{NewJobChunk, NewPreparedRemoteJob},
    server_connector::batch::{
        CaptureChunkReference, CaptureManifestReference, ContentIdentity,
        CreateRecordingJobRequest, ServerReplayKey, TranscriptResultRevision, UploadTrack,
    },
};

const CHUNK_PCM_BYTES: usize = 960_000;
const PCM_BYTES_PER_MILLISECOND: usize = 32;
const MAX_JOB_PCM_BYTES: u64 = 16_000 * 2 * 4 * 60 * 60;
const MAX_WAV_CONTAINER_OVERHEAD_BYTES: u64 = 1024 * 1024;
const MAX_RESULT_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;
const RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
static NEXT_STAGING_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub(super) struct PreparedRemoteChunk {
    pub(super) reference: CaptureChunkReference,
    pub(super) artifact_path: PathBuf,
}

pub(super) struct PreparedRemoteJob {
    pub(super) request: CreateRecordingJobRequest,
    pub(super) chunks: Vec<PreparedRemoteChunk>,
    pub(super) capture_manifest_path: PathBuf,
    pub(super) owner_namespace: String,
}

impl PreparedRemoteJob {
    pub(super) fn into_ledger_state(self) -> Result<NewPreparedRemoteJob, String> {
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

struct WavData {
    data_offset: u64,
    data_bytes: u64,
    source_bytes: u64,
}

pub(super) fn prepare_imported_pcm_wav(
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
    let nonce = NEXT_STAGING_DIRECTORY.fetch_add(1, Ordering::Relaxed);
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

fn inspect_pcm_wav(source: &mut File) -> Result<WavData, String> {
    let length = source
        .metadata()
        .map_err(|error| format!("failed to inspect imported WAV: {error}"))?
        .len();
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to seek imported WAV: {error}"))?;
    let mut header = [0_u8; 12];
    source
        .read_exact(&mut header)
        .map_err(|_| "imported recording is shorter than a WAV header".to_string())?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err("Phase 5 currently accepts canonical RIFF/WAVE input only".into());
    }
    let declared_end = u64::from(u32::from_le_bytes(header[4..8].try_into().unwrap())) + 8;
    if declared_end < 44 {
        return Err("imported WAV has an invalid RIFF length".into());
    }
    if declared_end != length {
        return Err("imported WAV file length does not match its RIFF boundary".into());
    }
    let mut format_valid = false;
    let mut data = None;
    let mut position = 12_u64;
    while position
        .checked_add(8)
        .is_some_and(|end| end <= declared_end)
    {
        source
            .seek(SeekFrom::Start(position))
            .map_err(|error| format!("failed to seek WAV chunk: {error}"))?;
        let mut chunk_header = [0_u8; 8];
        source
            .read_exact(&mut chunk_header)
            .map_err(|_| "imported WAV chunk header is truncated".to_string())?;
        let chunk_size = u64::from(u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()));
        let chunk_start = position + 8;
        let chunk_end = chunk_start
            .checked_add(chunk_size)
            .ok_or_else(|| "imported WAV chunk length overflowed".to_string())?;
        if chunk_end > declared_end || chunk_end > length {
            return Err("imported WAV chunk exceeds the RIFF boundary".into());
        }
        if &chunk_header[0..4] == b"fmt " {
            if chunk_size < 16 {
                return Err("imported WAV format chunk is truncated".into());
            }
            let mut format = [0_u8; 16];
            source
                .read_exact(&mut format)
                .map_err(|_| "imported WAV format chunk is truncated".to_string())?;
            format_valid = u16::from_le_bytes(format[0..2].try_into().unwrap()) == 1
                && u16::from_le_bytes(format[2..4].try_into().unwrap()) == 1
                && u32::from_le_bytes(format[4..8].try_into().unwrap()) == 16_000
                && u32::from_le_bytes(format[8..12].try_into().unwrap()) == 32_000
                && u16::from_le_bytes(format[12..14].try_into().unwrap()) == 2
                && u16::from_le_bytes(format[14..16].try_into().unwrap()) == 16;
        } else if &chunk_header[0..4] == b"data" {
            data = Some(WavData {
                data_offset: chunk_start,
                data_bytes: chunk_size,
                source_bytes: declared_end,
            });
        }
        position = chunk_end
            .checked_add(chunk_size % 2)
            .ok_or_else(|| "imported WAV padding overflowed".to_string())?;
    }
    if !format_valid {
        return Err("Phase 5 requires mono signed PCM16 WAV at 16 kHz".into());
    }
    let data = data.ok_or_else(|| "imported WAV has no data chunk".to_string())?;
    validate_pcm_data_bytes(data.data_bytes)?;
    let container_overhead = data
        .source_bytes
        .checked_sub(data.data_bytes)
        .ok_or_else(|| "imported WAV data exceeds its container".to_string())?;
    if container_overhead > MAX_WAV_CONTAINER_OVERHEAD_BYTES {
        return Err("imported WAV container metadata is too large".into());
    }
    Ok(data)
}

fn validate_pcm_data_bytes(data_bytes: u64) -> Result<(), String> {
    if data_bytes == 0 || !data_bytes.is_multiple_of(2) {
        return Err("imported WAV audio must contain whole PCM16 samples".into());
    }
    if data_bytes > MAX_JOB_PCM_BYTES {
        return Err("Phase 5 accepts at most four hours of PCM audio per recording".into());
    }
    Ok(())
}

fn prepare_spool_root(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("failed to create job spool: {error}"))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect job spool: {error}"))?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err("job spool must be a real directory".into());
    }
    Ok(())
}

pub(super) fn reset_unattached_spool(job_id: &str, spool_root: &Path) -> Result<(), String> {
    validate_identifier(job_id, 128, "job ID")?;
    prepare_spool_root(spool_root)?;
    let entries = fs::read_dir(spool_root)
        .map_err(|error| format!("failed to inspect job spool contents: {error}"))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| format!("failed to inspect job spool entry: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for entry in entries {
        let Some(name) = entry.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if owned_job_spool_entry(name, job_id) {
            quarantine_and_remove_job_spool_entry(&entry, job_id, spool_root)?;
        }
    }
    Ok(())
}

fn owned_job_spool_entry(name: &str, job_id: &str) -> bool {
    if name == job_id {
        return true;
    }
    let Some(suffix) = name.strip_prefix(&format!(".{job_id}-")) else {
        return false;
    };
    if let Some(staging) = suffix.strip_suffix(".part") {
        return decimal_pair(staging);
    }
    suffix.strip_prefix("orphan-").is_some_and(decimal_pair)
}

fn decimal_pair(value: &str) -> bool {
    let Some((left, right)) = value.split_once('-') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.bytes().all(|byte| byte.is_ascii_digit())
        && right.bytes().all(|byte| byte.is_ascii_digit())
}

fn quarantine_and_remove_job_spool_entry(
    source: &Path,
    job_id: &str,
    spool_root: &Path,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect prior job spool: {error}"))?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err("prior job spool is not a safe owned directory".into());
    }
    let quarantine = (0..1_024)
        .find_map(|_| {
            let nonce = NEXT_STAGING_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let candidate =
                spool_root.join(format!(".{job_id}-orphan-{}-{nonce}", std::process::id()));
            (candidate != source && !candidate.exists()).then_some(candidate)
        })
        .ok_or_else(|| "failed to reserve owned job spool quarantine".to_string())?;
    fs::rename(source, &quarantine)
        .map_err(|error| format!("failed to quarantine prior job spool: {error}"))?;
    let quarantined = fs::symlink_metadata(&quarantine)
        .map_err(|error| format!("failed to inspect quarantined job spool: {error}"))?;
    if !quarantined.is_dir() || metadata_is_link_or_reparse(&quarantined) {
        let _ = fs::rename(&quarantine, source);
        return Err("quarantined job spool changed type before cleanup".into());
    }
    fs::remove_dir_all(&quarantine)
        .map_err(|error| format!("failed to remove quarantined job spool: {error}"))
}

pub(super) fn read_prepared_chunk(
    artifact_path: &Path,
    spool_root: &Path,
    reference: &CaptureChunkReference,
) -> Result<Vec<u8>, String> {
    prepare_spool_root(spool_root)?;
    if !artifact_path.is_absolute() || !spool_root.is_absolute() {
        return Err("prepared chunk paths must be absolute".into());
    }
    let relative = artifact_path
        .strip_prefix(spool_root)
        .map_err(|_| "prepared chunk is outside the owned spool".to_string())?;
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 2
        || components
            .iter()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err("prepared chunk path has an invalid owned shape".into());
    }
    let parent = artifact_path
        .parent()
        .ok_or_else(|| "prepared chunk has no parent directory".to_string())?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("failed to inspect prepared chunk directory: {error}"))?;
    if !parent_metadata.is_dir() || metadata_is_link_or_reparse(&parent_metadata) {
        return Err("prepared chunk directory is not a safe owned directory".into());
    }
    let path_metadata = fs::symlink_metadata(artifact_path)
        .map_err(|error| format!("failed to inspect prepared chunk: {error}"))?;
    if !path_metadata.is_file() || metadata_is_link_or_reparse(&path_metadata) {
        return Err("prepared chunk must be a regular non-link file".into());
    }
    let expected_length = usize::try_from(reference.content_identity.byte_length)
        .map_err(|_| "prepared chunk length is out of range".to_string())?;
    if expected_length == 0 || expected_length > 1024 * 1024 {
        return Err("prepared chunk length is outside the server contract".into());
    }
    let mut file = open_no_follow_read(artifact_path)
        .map_err(|error| format!("failed to open prepared chunk: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect opened prepared chunk: {error}"))?;
    if !opened_metadata.is_file()
        || metadata_is_link_or_reparse(&opened_metadata)
        || opened_metadata.len() != reference.content_identity.byte_length
    {
        return Err("opened prepared chunk differs from its immutable declaration".into());
    }
    let mut body = Vec::with_capacity(expected_length);
    file.read_to_end(&mut body)
        .map_err(|error| format!("failed to read prepared chunk: {error}"))?;
    if body.len() != expected_length || sha256_bytes(&body) != reference.content_identity.sha256 {
        return Err("prepared chunk differs from its immutable content identity".into());
    }
    Ok(body)
}

pub(super) fn publish_remote_result(
    job_id: &str,
    spool_root: &Path,
    result: &TranscriptResultRevision,
) -> Result<PathBuf, String> {
    validate_identifier(job_id, 128, "job ID")?;
    validate_published_result_contract(result, 1)?;
    prepare_spool_root(spool_root)?;
    let job_root = spool_root.join(job_id);
    let job_metadata = fs::symlink_metadata(&job_root)
        .map_err(|error| format!("failed to inspect prepared job directory: {error}"))?;
    if !job_metadata.is_dir() || metadata_is_link_or_reparse(&job_metadata) {
        return Err("prepared job directory is not a safe owned directory".into());
    }
    let encoded_result = serde_json::to_vec(result)
        .map_err(|error| format!("failed to encode server result revision: {error}"))?;
    if encoded_result.len() > MAX_RESULT_ARTIFACT_BYTES {
        return Err("server result revision is too large to publish".into());
    }
    let mut transcript = result.transcript.as_bytes().to_vec();
    if !transcript.ends_with(b"\n") {
        transcript.push(b'\n');
    }
    if transcript.len() > MAX_RESULT_ARTIFACT_BYTES {
        return Err("server transcript is too large to publish".into());
    }

    let directory_name = format!("result-{:020}", result.revision);
    let destination = job_root.join(&directory_name);
    if destination.exists() {
        verify_published_remote_result(&destination, &encoded_result, &transcript)?;
        return Ok(destination.join("transcript.txt"));
    }

    let nonce = NEXT_STAGING_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let staging_path = job_root.join(format!(
        ".{directory_name}-staging-{}-{nonce}",
        std::process::id()
    ));
    let mut staging = StagingDirectory::create(staging_path)?;
    write_new_synced(&staging.path.join("result.json"), &encoded_result)?;
    write_new_synced(&staging.path.join("transcript.txt"), &transcript)?;
    match staging.publish(&destination) {
        Ok(()) => {}
        Err(_error) if destination.exists() => {
            verify_published_remote_result(&destination, &encoded_result, &transcript)?;
            return Ok(destination.join("transcript.txt"));
        }
        Err(error) => return Err(error),
    }
    Ok(destination.join("transcript.txt"))
}

pub(super) struct VerifiedRemoteTranscript {
    pub(super) result: TranscriptResultRevision,
    pub(super) text: String,
}

pub(super) fn read_published_remote_transcript(
    transcript_path: &Path,
    spool_root: &Path,
) -> Result<VerifiedRemoteTranscript, String> {
    let relative = transcript_path
        .strip_prefix(spool_root)
        .map_err(|_| "remote transcript is outside Yap's private job directory".to_string())?;
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 3 {
        return Err("remote transcript path has an invalid owned shape".into());
    }
    let job_id = normal_path_component(&components[0])
        .ok_or_else(|| "remote transcript job directory is invalid".to_string())?;
    let result_directory = normal_path_component(&components[1])
        .ok_or_else(|| "remote transcript result directory is invalid".to_string())?;
    let artifact_name = normal_path_component(&components[2])
        .ok_or_else(|| "remote transcript artifact name is invalid".to_string())?;
    validate_identifier(job_id, 128, "job ID")?;
    if artifact_name != "transcript.txt"
        || transcript_path
            != spool_root
                .join(job_id)
                .join(result_directory)
                .join("transcript.txt")
    {
        return Err("remote transcript path is not canonical".into());
    }
    let revision_text = result_directory
        .strip_prefix("result-")
        .filter(|value| value.len() == 20 && value.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| "remote transcript result revision is invalid".to_string())?;
    let revision = revision_text
        .parse::<u64>()
        .map_err(|_| "remote transcript result revision is invalid".to_string())?;
    if revision == 0 {
        return Err("remote transcript result revision is invalid".into());
    }

    for directory in [spool_root.to_path_buf(), spool_root.join(job_id)] {
        let metadata = fs::symlink_metadata(&directory)
            .map_err(|error| format!("failed to inspect remote result owner: {error}"))?;
        if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
            return Err("remote result owner is not a safe Yap directory".into());
        }
    }
    let destination = spool_root.join(job_id).join(result_directory);
    let destination_metadata = fs::symlink_metadata(&destination)
        .map_err(|error| format!("failed to inspect remote result revision: {error}"))?;
    if !destination_metadata.is_dir() || metadata_is_link_or_reparse(&destination_metadata) {
        return Err("remote result revision is not a safe Yap directory".into());
    }
    let result_path = destination.join("result.json");
    let result_bytes = read_bounded_regular_artifact(
        &result_path,
        MAX_RESULT_ARTIFACT_BYTES,
        "remote result revision",
    )?;
    let result: TranscriptResultRevision = serde_json::from_slice(&result_bytes)
        .map_err(|_| "remote result revision is incompatible".to_string())?;
    validate_published_result_contract(&result, revision)?;
    let mut expected_transcript = result.transcript.as_bytes().to_vec();
    if !expected_transcript.ends_with(b"\n") {
        expected_transcript.push(b'\n');
    }
    verify_published_remote_result(&destination, &result_bytes, &expected_transcript)?;
    let text = String::from_utf8(expected_transcript)
        .map_err(|_| "remote transcript is not valid UTF-8".to_string())?;
    Ok(VerifiedRemoteTranscript { result, text })
}

fn normal_path_component<'a>(component: &'a std::path::Component<'a>) -> Option<&'a str> {
    match component {
        std::path::Component::Normal(value) => value.to_str(),
        _ => None,
    }
}

fn read_bounded_regular_artifact(
    path: &Path,
    maximum_bytes: usize,
    label: &str,
) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {label}: {error}"))?;
    if !metadata.is_file()
        || metadata_is_link_or_reparse(&metadata)
        || metadata.len() > maximum_bytes as u64
    {
        return Err(format!("{label} is not a bounded regular Yap artifact"));
    }
    let mut file =
        open_no_follow_read(path).map_err(|error| format!("failed to open {label}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("failed to inspect opened {label}: {error}"))?;
    if !opened.is_file() || metadata_is_link_or_reparse(&opened) || opened.len() != metadata.len() {
        return Err(format!("opened {label} differs from its owned path"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read {label}: {error}"))?;
    if bytes.len() != metadata.len() as usize {
        return Err(format!("{label} changed while it was read"));
    }
    Ok(bytes)
}

fn validate_published_result_contract(
    result: &TranscriptResultRevision,
    expected_revision: u64,
) -> Result<(), String> {
    validate_identifier(&result.session_id, 128, "result session ID")?;
    let timestamp_valid = result.created_at_utc.ends_with('Z')
        && result.created_at_utc.len() <= 64
        && OffsetDateTime::parse(&result.created_at_utc, &Rfc3339).is_ok();
    let language_valid = result.language.as_ref().is_some_and(|language| {
        !language.language_bcp47.is_empty()
            && language.language_bcp47.len() <= 35
            && language
                .language_bcp47
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && language
                .confidence
                .is_none_or(|confidence| (0.0..=1.0).contains(&confidence))
    });
    let provenance_valid = !result.model_provenance.is_empty()
        && result.model_provenance.len() <= 8
        && result.model_provenance.iter().all(|model| {
            [
                model.model_id.as_str(),
                model.revision.as_str(),
                model.calibration_revision.as_str(),
            ]
            .iter()
            .all(|value| !value.is_empty() && value.len() <= 256)
        });
    if result.revision != expected_revision
        || result.authority != "server_authoritative"
        || !timestamp_valid
        || !valid_sha256(&result.capture_manifest_sha256)
        || result
            .previous_result_sha256
            .as_deref()
            .is_some_and(|value| !valid_sha256(value))
        || result.status != "complete"
        || !language_valid
        || result.transcript.trim().is_empty()
        || result.transcript.len() > MAX_RESULT_ARTIFACT_BYTES - 1
        || !result.aligned_words.is_empty()
        || !provenance_valid
    {
        return Err("remote result revision conflicts with the published transcript".into());
    }
    Ok(())
}

fn verify_published_remote_result(
    destination: &Path,
    expected_result: &[u8],
    expected_transcript: &[u8],
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(destination)
        .map_err(|error| format!("failed to inspect published result directory: {error}"))?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err("published result path is not a safe owned directory".into());
    }
    let mut names = fs::read_dir(destination)
        .map_err(|error| format!("failed to inspect published result contents: {error}"))?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .map_err(|error| format!("failed to inspect published result entry: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    if names != ["result.json", "transcript.txt"] {
        return Err("published result directory has unexpected contents".into());
    }
    for (name, expected) in [
        ("result.json", expected_result),
        ("transcript.txt", expected_transcript),
    ] {
        let path = destination.join(name);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect published result artifact: {error}"))?;
        if !metadata.is_file()
            || metadata_is_link_or_reparse(&metadata)
            || metadata.len() != expected.len() as u64
        {
            return Err("published result artifact conflicts with its declaration".into());
        }
        let mut file = open_no_follow_read(&path)
            .map_err(|error| format!("failed to open published result artifact: {error}"))?;
        let mut actual = Vec::with_capacity(expected.len());
        file.read_to_end(&mut actual)
            .map_err(|error| format!("failed to read published result artifact: {error}"))?;
        if actual != expected {
            return Err("published result artifact conflicts with its immutable content".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn open_no_follow_read(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(target_os = "linux")]
fn open_no_follow_read(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x0002_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

#[cfg(target_os = "macos")]
fn open_no_follow_read(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x0000_0100;
    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn open_no_follow_read(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure no-follow chunk open is unsupported on this platform",
    ))
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

struct StagingDirectory {
    path: PathBuf,
    published: bool,
}

impl StagingDirectory {
    fn create(path: PathBuf) -> Result<Self, String> {
        fs::create_dir(&path)
            .map_err(|error| format!("failed to reserve job spool staging: {error}"))?;
        Ok(Self {
            path,
            published: false,
        })
    }

    fn publish(&mut self, destination: &Path) -> Result<(), String> {
        fs::rename(&self.path, destination)
            .map_err(|error| format!("failed to publish prepared job spool: {error}"))?;
        self.published = true;
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn write_new_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("failed to create prepared job artifact: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write prepared job artifact: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush prepared job artifact: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync prepared job artifact: {error}"))
}

fn sha256_reader(file: &mut File, expected_bytes: u64) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to seek imported recording: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut remaining = expected_bytes;
    while remaining > 0 {
        let requested = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| "imported recording hash length is out of range")?;
        let read = file
            .read(&mut buffer[..requested])
            .map_err(|error| format!("failed to hash imported recording: {error}"))?;
        if read == 0 {
            return Err("imported recording changed while it was being hashed".into());
        }
        digest.update(&buffer[..read]);
        remaining -= read as u64;
    }
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|error| format!("failed to verify imported recording length: {error}"))?
        != 0
    {
        return Err("imported recording changed while it was being hashed".into());
    }
    Ok(format_digest(digest.finalize().as_slice()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format_digest(Sha256::digest(bytes).as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_identifier(value: &str, maximum: usize, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > maximum
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, File},
        io::Write,
        time::{Duration, UNIX_EPOCH},
    };

    use crate::{
        audio::session::OwnerNamespace,
        server_connector::batch::{LanguageDecision, ModelRevision, TranscriptResultRevision},
    };

    use super::{
        prepare_imported_pcm_wav, publish_remote_result, read_prepared_chunk,
        read_published_remote_transcript, reset_unattached_spool, validate_pcm_data_bytes,
        validate_published_result_contract,
    };

    #[test]
    fn client_intake_matches_the_server_four_hour_pcm_ceiling() {
        let four_hours = 16_000_u64 * 2 * 4 * 60 * 60;
        assert!(validate_pcm_data_bytes(four_hours).is_ok());
        assert!(validate_pcm_data_bytes(four_hours + 2).is_err());
    }

    #[test]
    fn wav_bytes_outside_declared_riff_are_rejected_before_spooling() {
        let root =
            std::env::temp_dir().join(format!("yap-phase5-riff-boundary-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source_path = root.join("source.wav");
        write_pcm_wav(&source_path, &[0_u8; 320]);
        let mut append = fs::OpenOptions::new()
            .append(true)
            .open(&source_path)
            .unwrap();
        append.write_all(b"private trailing bytes").unwrap();
        append.sync_all().unwrap();
        drop(append);
        let mut source = File::open(&source_path).unwrap();
        let owner = OwnerNamespace::local("i-phase5-riff-boundary").unwrap();

        let error = prepare_imported_pcm_wav(
            "job-phase5-riff-boundary",
            "source.wav",
            &mut source,
            &root.join("spool"),
            &owner,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .err()
        .expect("trailing bytes must reject the imported WAV");

        assert_eq!(
            error,
            "imported WAV file length does not match its RIFF boundary"
        );
        assert!(!root.join("spool").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_wav_container_metadata_is_rejected_before_spooling() {
        let root =
            std::env::temp_dir().join(format!("yap-phase5-riff-overhead-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source_path = root.join("source.wav");
        write_pcm_wav_with_junk(
            &source_path,
            &[0_u8; 320],
            super::MAX_WAV_CONTAINER_OVERHEAD_BYTES as usize,
        );
        let mut source = File::open(&source_path).unwrap();
        let owner = OwnerNamespace::local("i-phase5-riff-overhead").unwrap();

        let error = prepare_imported_pcm_wav(
            "job-phase5-riff-overhead",
            "source.wav",
            &mut source,
            &root.join("spool"),
            &owner,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .err()
        .expect("oversized WAV metadata must be rejected");

        assert_eq!(error, "imported WAV container metadata is too large");
        assert!(!root.join("spool").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_pcm_wav_becomes_an_immutable_owned_upload_manifest() {
        let root = std::env::temp_dir().join(format!("yap-phase5-prepare-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let source_path = root.join("source.wav");
        let pcm = vec![0_u8; 320];
        write_pcm_wav(&source_path, &pcm);
        let original = fs::read(&source_path).unwrap();
        let mut source = File::open(&source_path).unwrap();
        let owner = OwnerNamespace::local("i-phase5-test").unwrap();

        let prepared = prepare_imported_pcm_wav(
            "job-phase5-test",
            "source.wav",
            &mut source,
            &root.join("spool"),
            &owner,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();

        assert_eq!(prepared.request.route, "server_batch");
        assert_eq!(
            prepared.request.metadata.origin,
            crate::audio::session::SessionOrigin::ImportedFile
        );
        assert_eq!(
            prepared.request.metadata.preferred_languages_bcp47,
            ["en-US"]
        );
        assert_eq!(prepared.request.tracks.len(), 1);
        assert_eq!(prepared.request.chunks.len(), 1);
        assert_eq!(prepared.chunks.len(), 1);
        assert_eq!(fs::read(&prepared.chunks[0].artifact_path).unwrap(), pcm);
        assert!(prepared.capture_manifest_path.is_file());
        assert_eq!(
            fs::metadata(&prepared.capture_manifest_path).unwrap().len(),
            prepared.request.capture_manifest.byte_length
        );
        assert_eq!(fs::read(source_path).unwrap(), original);
        assert_eq!(prepared.owner_namespace, owner.as_str());
        assert_eq!(
            read_prepared_chunk(
                &prepared.chunks[0].artifact_path,
                &root.join("spool"),
                &prepared.chunks[0].reference,
            )
            .unwrap(),
            pcm
        );

        let durable = prepared.into_ledger_state().unwrap();
        let durable_request: serde_json::Value =
            serde_json::from_str(&durable.create_request_json).unwrap();
        assert_eq!(durable_request["route"], "server_batch");
        assert_eq!(durable.chunks.len(), 1);
        assert_eq!(durable.chunks[0].content_byte_length, 320);
        assert_eq!(durable.chunks[0].sequence_start, 0);
        assert_eq!(durable.chunks[0].sequence_end, 159);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cleanup_removes_only_exact_owned_job_staging_shapes_after_a_crash() {
        let root =
            std::env::temp_dir().join(format!("yap-phase5-staging-cleanup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let spool = root.join("remote-jobs");
        fs::create_dir_all(&spool).unwrap();
        let abandoned_prepare = spool.join(".job-stale-4242-7.part");
        let abandoned_quarantine = spool.join(".job-stale-orphan-4242-8");
        let unrelated = spool.join(".job-stale-user-data");
        for directory in [&abandoned_prepare, &abandoned_quarantine, &unrelated] {
            fs::create_dir(directory).unwrap();
            fs::write(directory.join("private.pcm"), b"private bytes").unwrap();
        }

        reset_unattached_spool("job-stale", &spool).unwrap();

        assert!(!abandoned_prepare.exists());
        assert!(!abandoned_quarantine.exists());
        assert!(unrelated.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn published_remote_transcript_is_reopened_only_through_its_result_revision() {
        let root =
            std::env::temp_dir().join(format!("yap-phase5-result-open-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let spool = root.join("remote-jobs");
        let job_id = "job-phase5-result-open";
        fs::create_dir_all(spool.join(job_id)).unwrap();
        let result = TranscriptResultRevision {
            session_id: "s-phase5-result-open".into(),
            revision: 1,
            authority: "server_authoritative".into(),
            created_at_utc: "2026-07-14T21:00:02Z".into(),
            capture_manifest_sha256:
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            previous_result_sha256: None,
            status: "complete".into(),
            language: Some(LanguageDecision {
                language_bcp47: "en-US".into(),
                confidence: Some(0.98),
            }),
            transcript: "Private result.".into(),
            aligned_words: Vec::new(),
            model_provenance: vec![ModelRevision {
                model_id: "CohereLabs/cohere-transcribe-03-2026".into(),
                revision: "b1eacc2686a3d08ceaae5f24a88b1d519620bc09".into(),
                calibration_revision: "asr-not-applicable".into(),
            }],
        };

        let output = publish_remote_result(job_id, &spool, &result).unwrap();
        let reopened = read_published_remote_transcript(&output, &spool).unwrap();
        assert_eq!(reopened.text, "Private result.\n");
        assert_eq!(reopened.result, result);

        let mut empty = result.clone();
        empty.transcript = " \n\t".into();
        assert!(validate_published_result_contract(&empty, 1).is_err());
        assert!(publish_remote_result(job_id, &spool, &empty).is_err());
        let mut offset_timestamp = result.clone();
        offset_timestamp.created_at_utc = "2026-07-14T16:00:02-05:00".into();
        assert!(validate_published_result_contract(&offset_timestamp, 1).is_err());

        fs::write(&output, "tampered\n").unwrap();
        assert!(read_published_remote_transcript(&output, &spool).is_err());
        assert!(read_published_remote_transcript(
            &spool
                .join(job_id)
                .join("result-00000000000000000001/../transcript.txt"),
            &spool,
        )
        .is_err());

        fs::remove_dir_all(root).unwrap();
    }

    fn write_pcm_wav(path: &std::path::Path, pcm: &[u8]) {
        let mut file = File::create(path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36_u32 + pcm.len() as u32).to_le_bytes())
            .unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16_u32.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&16_000_u32.to_le_bytes()).unwrap();
        file.write_all(&32_000_u32.to_le_bytes()).unwrap();
        file.write_all(&2_u16.to_le_bytes()).unwrap();
        file.write_all(&16_u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&(pcm.len() as u32).to_le_bytes()).unwrap();
        file.write_all(pcm).unwrap();
        file.sync_all().unwrap();
    }

    fn write_pcm_wav_with_junk(path: &std::path::Path, pcm: &[u8], junk_bytes: usize) {
        let file_bytes = 52_u64 + junk_bytes as u64 + pcm.len() as u64;
        let mut file = File::create(path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&u32::try_from(file_bytes - 8).unwrap().to_le_bytes())
            .unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16_u32.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&16_000_u32.to_le_bytes()).unwrap();
        file.write_all(&32_000_u32.to_le_bytes()).unwrap();
        file.write_all(&2_u16.to_le_bytes()).unwrap();
        file.write_all(&16_u16.to_le_bytes()).unwrap();
        file.write_all(b"JUNK").unwrap();
        file.write_all(&u32::try_from(junk_bytes).unwrap().to_le_bytes())
            .unwrap();
        file.write_all(&vec![0_u8; junk_bytes]).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&u32::try_from(pcm.len()).unwrap().to_le_bytes())
            .unwrap();
        file.write_all(pcm).unwrap();
        file.sync_all().unwrap();
    }
}
