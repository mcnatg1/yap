use super::{
    artifact_io::{
        metadata_is_link_or_reparse, next_staging_nonce, open_no_follow_read, valid_sha256,
        validate_identifier, write_new_synced, StagingDirectory,
    },
    spool::prepare_spool_root,
};
use crate::server_connector::batch::TranscriptResultRevision;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const MAX_RESULT_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;

pub(in crate::jobs) fn publish_remote_result(
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

    let nonce = next_staging_nonce();
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

pub(in crate::jobs) struct VerifiedRemoteTranscript {
    pub(in crate::jobs) result: TranscriptResultRevision,
    pub(in crate::jobs) text: String,
}

pub(in crate::jobs) fn read_published_remote_transcript(
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

pub(super) fn validate_published_result_contract(
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
