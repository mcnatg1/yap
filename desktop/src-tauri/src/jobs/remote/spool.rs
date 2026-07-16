use super::artifact_io::{metadata_is_link_or_reparse, next_staging_nonce, validate_identifier};
use std::{fs, path::Path};

pub(super) fn prepare_spool_root(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("failed to create job spool: {error}"))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect job spool: {error}"))?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err("job spool must be a real directory".into());
    }
    Ok(())
}

pub(in crate::jobs) fn reset_unattached_spool(
    job_id: &str,
    spool_root: &Path,
) -> Result<(), String> {
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
            let nonce = next_staging_nonce();
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
