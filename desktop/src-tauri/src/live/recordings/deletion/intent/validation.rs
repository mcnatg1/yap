use std::{collections::HashSet, path::Path};

use time::OffsetDateTime;

use crate::audio::recording;

use super::super::super::retention::committed_meeting_is_expired;
use super::super::evidence::physical_entry_exists;
use super::model::{DeletionIntent, DELETION_INTENT_SCHEMA_VERSION, MAX_DELETION_ARTIFACTS};

pub(in crate::live::recordings::deletion) fn revalidate_intent_artifact(
    dir: &Path,
    name: &str,
    sha256: &str,
    identity: Option<&recording::FileIdentity>,
) -> Result<(), String> {
    let identity = identity.ok_or_else(|| {
        "recording deletion intent lacks durable identity evidence; manual reconciliation is required"
            .to_string()
    })?;
    recording::revalidate_regular_artifact_file_identity_and_hash(dir, name, identity, sha256)
}

pub(in crate::live::recordings::deletion) fn prove_intent_against_current_commit(
    dir: &Path,
    intent: &DeletionIntent,
    now: OffsetDateTime,
) -> Result<(), String> {
    let (commit_text, commit_sha256) =
        recording::read_and_hash_regular_artifact(dir, &intent.commit_file)?;
    if commit_sha256 != intent.commit_sha256 {
        return Err("recording deletion commit no longer matches the published intent".into());
    }
    let manifest: recording::CaptureCommitManifest = serde_json::from_str(&commit_text)
        .map_err(|error| format!("Failed to parse committed recording manifest: {error}"))?;
    manifest.validate()?;
    if manifest.session_id != intent.session_id
        || !intent_has_artifact(intent, &manifest.audio_file, &manifest.audio_sha256)
        || !intent_has_artifact(
            intent,
            &manifest.capture_sidecar_file,
            &manifest.capture_sidecar_sha256,
        )
    {
        return Err(
            "recording deletion intent does not match the current committed session".into(),
        );
    }
    if intent.reason == "expired-meeting-retention" && !committed_meeting_is_expired(&manifest, now)
    {
        return Err(
            "expired-meeting deletion is no longer authorized by the committed metadata".into(),
        );
    }
    Ok(())
}

fn intent_has_artifact(intent: &DeletionIntent, name: &str, sha256: &str) -> bool {
    intent
        .artifacts
        .iter()
        .any(|artifact| artifact.name == name && artifact.sha256 == sha256)
}

pub(in crate::live::recordings::deletion) fn intent_originals_are_intact(
    dir: &Path,
    intent: &DeletionIntent,
) -> Result<bool, String> {
    if !physical_entry_exists(dir, &intent.commit_file)? {
        return Ok(false);
    }
    if intent.reason != "recoverable"
        && prove_intent_against_current_commit(dir, intent, OffsetDateTime::now_utc()).is_err()
    {
        return Ok(false);
    }
    if revalidate_intent_artifact(
        dir,
        &intent.commit_file,
        &intent.commit_sha256,
        intent.commit_file_identity.as_ref(),
    )
    .is_err()
    {
        return Ok(false);
    }
    for artifact in &intent.artifacts {
        if !physical_entry_exists(dir, &artifact.name)?
            || revalidate_intent_artifact(
                dir,
                &artifact.name,
                &artifact.sha256,
                artifact.file_identity.as_ref(),
            )
            .is_err()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(in crate::live::recordings) fn validate_deletion_intent(
    intent: &DeletionIntent,
) -> Result<(), String> {
    let recoverable = intent.reason == "recoverable";
    if intent.schema_version != DELETION_INTENT_SCHEMA_VERSION
        || (!recoverable
            && intent.reason != "manual"
            && intent.reason != "expired-meeting-retention")
        || (!recoverable && intent.commit_file != format!("live-{}.commit.json", intent.session_id))
        || (recoverable
            && !is_deletion_artifact_for_session(&intent.commit_file, &intent.session_id))
        || (!recoverable && intent.artifacts.is_empty())
        || intent.artifacts.len() > MAX_DELETION_ARTIFACTS
        || !is_sha256(&intent.commit_sha256)
    {
        return Err("recording deletion intent has an unsupported shape".into());
    }
    let mut names = HashSet::new();
    for artifact in &intent.artifacts {
        recording::validate_artifact_name(&artifact.name)?;
        if !names.insert(artifact.name.clone())
            || artifact.name == intent.commit_file
            || !is_deletion_artifact_for_session(&artifact.name, &intent.session_id)
        {
            return Err("recording deletion intent names an unowned artifact".into());
        }
        if !is_sha256(&artifact.sha256) {
            return Err("recording deletion intent has an invalid artifact hash".into());
        }
    }
    if intent.commit_file_identity.is_none()
        || intent
            .artifacts
            .iter()
            .any(|artifact| artifact.file_identity.is_none())
    {
        return Err(
            "recording deletion intent lacks complete durable identity evidence; manual reconciliation is required"
                .into(),
        );
    }
    if !recoverable
        && (!names.contains(&format!("live-{}.wav", intent.session_id))
            || !names.contains(&format!("live-{}.capture.json", intent.session_id)))
    {
        return Err("recording deletion intent is missing required capture artifacts".into());
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_deletion_artifact_for_session(
    name: &str,
    session_id: &crate::audio::session::SessionId,
) -> bool {
    let stem = format!("live-{session_id}");
    name == format!("{stem}.wav")
        || name == format!("{stem}.wav.part")
        || name == format!("{stem}.capture.json")
        || name == format!("{stem}.capture.json.part")
        || name == format!("{stem}.capture.journal.part")
        || name == format!("{stem}.capture.partial.json")
        || name == format!("{stem}.capture.partial.json.part")
        || name == format!("{stem}.commit.json")
        || name == format!("{stem}.commit.json.part")
        || name == format!("{stem}.txt")
        || name == format!("{stem}.polished.txt")
        || name
            .strip_prefix(&format!("{stem}.transcript.r"))
            .and_then(|value| value.strip_suffix(".json"))
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|revision| revision > 0)
}
