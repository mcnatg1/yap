use std::path::Path;
use std::time::Duration;

use crate::audio::recording;

#[cfg(test)]
use super::super::mutation_ownership::session_mutation_ownership;
use super::super::transcripts::{system_time_to_unix_millis, unix_millis_now};

const PRIVATE_DELETION_LEFTOVER_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub(super) fn physical_entry_exists(dir: &Path, name: &str) -> Result<bool, String> {
    recording::validate_artifact_name(name)?;
    match std::fs::symlink_metadata(dir.join(name)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Failed to inspect recording deletion artifact: {error}"
        )),
    }
}

pub(super) fn private_process_id(value: &str) -> Option<u32> {
    let (process_id, nonce) = value.split_once('-')?;
    if nonce.contains('-') {
        return None;
    }
    nonce.parse::<u64>().ok()?;
    process_id.parse::<u32>().ok()
}

fn session_from_yap_artifact(name: &str) -> Option<crate::audio::session::SessionId> {
    let session = name.strip_prefix("live-")?;
    [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json.part",
        ".capture.partial.json",
        ".commit.json.part",
        ".deletion.v1.json",
        ".commit.json",
        ".capture.json",
        ".polished.txt",
        ".wav",
        ".txt",
    ]
    .into_iter()
    .find_map(|suffix| session.strip_suffix(suffix))
    .and_then(|session| crate::audio::session::SessionId::new(session.to_string()).ok())
    .or_else(|| {
        let (session, revision) = session.rsplit_once(".transcript.r")?;
        revision
            .strip_suffix(".json")?
            .parse::<u64>()
            .ok()
            .filter(|revision| *revision > 0)?;
        crate::audio::session::SessionId::new(session.to_string()).ok()
    })
}

pub(super) fn generic_delete_quarantine(name: &str) -> Option<(&str, u32)> {
    let (artifact, suffix) = name.strip_prefix('.')?.rsplit_once(".delete-")?;
    session_from_yap_artifact(artifact)?;
    Some((artifact, private_process_id(suffix)?))
}

fn intent_evidence_quarantine_is_reconcilable(
    dir: &Path,
    name: &str,
    process_id: u32,
) -> Result<bool, String> {
    if process_id == std::process::id() {
        Ok(true)
    } else {
        private_deletion_leftover_is_old(dir, name)
    }
}

#[cfg(test)]
pub(in crate::live::recordings) fn reconcile_intent_evidence_quarantines(
    dir: &Path,
    intent_name: &str,
) -> Result<(), String> {
    let _ownership = session_mutation_ownership();
    reconcile_intent_evidence_quarantines_while_owned(dir, intent_name)
}

pub(super) fn reconcile_intent_evidence_quarantines_while_owned(
    dir: &Path,
    intent_name: &str,
) -> Result<(), String> {
    let mut newest = None;
    for entry in std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to scan recording deletion evidence: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("Failed to inspect recording deletion evidence: {error}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some((artifact_name, process_id)) = generic_delete_quarantine(&name) else {
            continue;
        };
        if artifact_name != intent_name
            || !intent_evidence_quarantine_is_reconcilable(dir, &name, process_id)?
        {
            continue;
        }
        let file = recording::open_regular_artifact(dir, &name)?;
        let modified = file
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map_err(|error| format!("Failed to inspect recording deletion evidence: {error}"))?;
        if newest.as_ref().is_none_or(
            |(current, current_modified): &(String, std::time::SystemTime)| {
                modified > *current_modified || (modified == *current_modified && name > *current)
            },
        ) {
            newest = Some((name, modified));
        }
    }
    let Some((newest, _)) = newest else {
        return Ok(());
    };
    if !physical_entry_exists(dir, intent_name)? {
        let artifact = recording::verified_regular_artifact(dir, &newest)?;
        recording::restore_verified_quarantined_artifact(&artifact, &dir.join(intent_name))?;
        return reconcile_intent_evidence_quarantines_while_owned(dir, intent_name);
    }
    for entry in std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to scan recording deletion evidence: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("Failed to inspect recording deletion evidence: {error}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some((artifact_name, process_id)) = generic_delete_quarantine(&name) else {
            continue;
        };
        if artifact_name == intent_name
            && intent_evidence_quarantine_is_reconcilable(dir, &name, process_id)?
        {
            let artifact = recording::verified_regular_artifact(dir, &name)?;
            recording::remove_verified_quarantined_artifact(&artifact)?;
        }
    }
    Ok(())
}

pub(super) fn private_deletion_leftover_is_old(dir: &Path, name: &str) -> Result<bool, String> {
    let file = recording::open_regular_artifact(dir, name)?;
    let modified = file
        .metadata()
        .map_err(|error| format!("Failed to inspect private deletion artifact: {error}"))?
        .modified()
        .map_err(|error| format!("Failed to inspect private deletion artifact age: {error}"))?;
    let modified = system_time_to_unix_millis(modified)
        .ok_or_else(|| "Private deletion artifact has an invalid modification time".to_string())?;
    let now = unix_millis_now()?;
    Ok(now.saturating_sub(modified) >= PRIVATE_DELETION_LEFTOVER_TTL.as_millis() as u64)
}
