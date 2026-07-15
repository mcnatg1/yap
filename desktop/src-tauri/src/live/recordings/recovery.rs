use std::collections::BTreeSet;

use super::deletion::{
    admit_deletion_artifact, admit_expected_private_artifact_identity, deletion_intent_name,
    push_maintenance_warning, resume_deletion_intent_while_owned, validate_deletion_intent,
    write_deletion_intent_with_publication_barrier_while_owned, DeletionIntent,
    DELETION_INTENT_SCHEMA_VERSION,
};
use super::*;

#[cfg(test)]
pub(super) fn list_recoverable_live_sessions_from_dir(
    dir: &Path,
) -> Result<Vec<RecoverableLiveSession>, String> {
    let _ownership = session_mutation_ownership();
    list_recoverable_live_sessions_from_scan(
        dir,
        &recording::scan_recordings(dir)?,
        OffsetDateTime::now_utc(),
    )
}

pub(super) fn list_recoverable_live_sessions_from_scan(
    dir: &Path,
    scan: &recording::RecordingScan,
    now: OffsetDateTime,
) -> Result<Vec<RecoverableLiveSession>, String> {
    let mut sessions = Vec::new();
    for recovered in &scan.recovered_partial {
        if let Some(saved) = saved_recovered_session(dir, &recovered.session_id)? {
            sessions.push(RecoverableLiveSession {
                session_id: recovered.session_id.to_string(),
                name: saved.name,
                audio_partial_path: Some(saved.source_path),
                journal_partial_path: regular_artifact_exists(
                    dir,
                    &format!("live-{}.capture.journal.part", recovered.session_id),
                )
                .then(|| {
                    stable_existing_path_string(&dir.join(format!(
                        "live-{}.capture.journal.part",
                        recovered.session_id
                    )))
                }),
                reason: "Recovered partial recording is retained for recovery or deletion.".into(),
                expires_at_ms: u64::MAX,
            });
        }
    }
    for partial in &scan.partial {
        let Some(session_id) = partial.session_id.as_ref() else {
            continue;
        };
        let candidate = recoverable_session_from_dir(dir, session_id)?;
        let now_ms = u64::try_from(now.unix_timestamp_nanos() / 1_000_000).unwrap_or(u64::MAX);
        if candidate.expires_at_ms <= now_ms {
            let cleanup = delete_recoverable_session_artifacts_while_owned(dir, session_id, None);
            if let Err(error) = cleanup {
                sessions.push(RecoverableLiveSession {
                    reason: format!("{} Cleanup is pending: {error}", candidate.reason),
                    ..candidate
                });
            }
            continue;
        }
        sessions.push(candidate);
    }
    Ok(sessions)
}

pub(super) fn damaged_commit_warnings(
    scan: &recording::RecordingScan,
    warnings: Vec<String>,
) -> Vec<String> {
    let mut prioritized = Vec::new();
    for damaged in &scan.damaged {
        push_maintenance_warning(
            &mut prioritized,
            format!(
                "Damaged live recording {} was preserved: {}",
                damaged.session_id, damaged.reason
            ),
        );
    }
    for warning in warnings {
        push_maintenance_warning(&mut prioritized, warning);
    }
    prioritized
}

pub(super) fn recoverable_session_from_dir(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<RecoverableLiveSession, String> {
    let name = format!("live-{session_id}");
    let audio_name = format!("{name}.wav.part");
    let journal_name = format!("{name}.capture.journal.part");
    let orphan_audio_name = format!("{name}.wav");
    let audio = regular_artifact_exists(dir, &audio_name)
        .then(|| stable_existing_path_string(&dir.join(&audio_name)))
        .or_else(|| {
            regular_artifact_exists(dir, &orphan_audio_name)
                .then(|| stable_existing_path_string(&dir.join(&orphan_audio_name)))
        });
    let journal = regular_artifact_exists(dir, &journal_name)
        .then(|| stable_existing_path_string(&dir.join(&journal_name)));
    let recorded_at = [
        audio_name.as_str(),
        orphan_audio_name.as_str(),
        journal_name.as_str(),
    ]
    .into_iter()
    .filter_map(|artifact| artifact_modified_at_ms(dir, artifact))
    .min()
    .unwrap_or_else(|| unix_millis_now().unwrap_or(0));
    let expires_at_ms = recorded_at.saturating_add(PARTIAL_RECOVERY_TTL.as_millis() as u64);
    Ok(RecoverableLiveSession {
        session_id: session_id.to_string(),
        name,
        audio_partial_path: audio,
        journal_partial_path: journal,
        reason: "Recording did not finish and can be recovered or deleted.".into(),
        expires_at_ms,
    })
}

pub(super) fn recoverable_session_artifact_path(session: &RecoverableLiveSession) -> Option<&str> {
    session
        .audio_partial_path
        .as_deref()
        .or(session.journal_partial_path.as_deref())
}

pub(super) fn saved_session_action_artifact_path(session: &SavedLiveSession) -> &str {
    if session.recovery_state.is_some() {
        &session.source_path
    } else {
        &session.output_path
    }
}

pub(super) fn recover_live_session_in_dir(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
) -> Result<SavedLiveSession, String> {
    recover_live_session_in_dir_with_mutation_barrier(
        dir,
        session_id,
        expected_artifact_path,
        || {},
    )
}

pub(super) fn recover_live_session_in_dir_with_mutation_barrier<F>(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
    mutation_barrier: F,
) -> Result<SavedLiveSession, String>
where
    F: FnOnce(),
{
    recover_live_session_in_dir_with_queue_observer(
        dir,
        session_id,
        expected_artifact_path,
        || {},
        mutation_barrier,
    )
}

pub(super) fn recover_live_session_in_dir_with_queue_observer<F, Q>(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
    queued: Q,
    mutation_barrier: F,
) -> Result<SavedLiveSession, String>
where
    F: FnOnce(),
    Q: FnOnce(),
{
    let _ownership = session_mutation_ownership_with_queue_observer(queued);
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    if let Some(saved) = saved_recovered_session(dir, &session_id)? {
        let expected = admit_expected_private_artifact_identity(
            Path::new(saved_session_action_artifact_path(&saved)),
            &expected_artifact_path,
        )?;
        mutation_barrier();
        recording::revalidate_regular_artifact_identity(&expected)?;
        return Ok(saved);
    }
    ensure_recoverable_session(dir, &session_id)?;
    let candidate = recoverable_session_from_dir(dir, &session_id)?;
    let artifact_path = recoverable_session_artifact_path(&candidate).ok_or_else(|| {
        "Recoverable live recording has no authoritative artifact identity.".to_string()
    })?;
    let expected = admit_expected_private_artifact_identity(
        Path::new(artifact_path),
        &expected_artifact_path,
    )?;
    if candidate.expires_at_ms <= unix_millis_now()? {
        return Err("Recoverable live recording has expired.".into());
    }
    if candidate.audio_partial_path.is_none() {
        return Err("Recoverable live recording has no safe partial WAV to repair.".into());
    }
    mutation_barrier();
    let (audio_file, audio_bytes, audio_sha256) =
        recording::recover_partial_wav_with_identity(dir, &session_id, &expected)?;
    let sidecar_file = format!("live-{session_id}.capture.partial.json");
    let sidecar_sha256 = match recording::read_and_hash_regular_artifact(dir, &sidecar_file) {
        Ok((text, hash)) if valid_partial_sidecar(&text, &session_id) => hash,
        Ok(_) => return Err("Partial capture sidecar does not match the recovery session.".into()),
        Err(_) => {
            let sidecar = serde_json::json!({
                "schemaVersion": 1u16,
                "sessionId": session_id,
                "status": "partial",
            });
            let receipt = write_new_text_file(
                &dir.join(&sidecar_file),
                &format!(
                    "{}\n",
                    serde_json::to_string(&sidecar).map_err(|error| error.to_string())?
                ),
            )?;
            receipt.sha256().to_string()
        }
    };
    let commit_file = format!("live-{session_id}.commit.json");
    let commit = recording::PartialRecoveryCommit {
        schema_version: 1,
        session_id: session_id.clone(),
        status: recording::CaptureStatus::Partial,
        audio_file: audio_file.clone(),
        audio_sha256,
        audio_bytes,
        capture_sidecar_file: sidecar_file,
        capture_sidecar_sha256: sidecar_sha256,
        committed_at_utc: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| "Failed to format recovery timestamp")?,
    };
    write_new_text_file(
        &dir.join(&commit_file),
        &format!(
            "{}\n",
            serde_json::to_string(&commit).map_err(|error| error.to_string())?
        ),
    )?;
    saved_recovered_session(dir, &session_id)?
        .ok_or_else(|| "Recovered live recording was not verifiable.".into())
}

pub(super) fn delete_recoverable_live_session_in_dir(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
) -> Result<(), String> {
    delete_recoverable_live_session_in_dir_with_mutation_barrier(
        dir,
        session_id,
        expected_artifact_path,
        || {},
    )
}

pub(super) fn delete_recoverable_live_session_in_dir_with_mutation_barrier<F>(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
    mutation_barrier: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    let _ownership = session_mutation_ownership();
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    ensure_recoverable_session(dir, &session_id)?;
    let candidate = recoverable_session_from_dir(dir, &session_id)?;
    let artifact_path = recoverable_session_artifact_path(&candidate).ok_or_else(|| {
        "Recoverable live recording has no authoritative artifact identity.".to_string()
    })?;
    let expected = admit_expected_private_artifact_identity(
        Path::new(artifact_path),
        &expected_artifact_path,
    )?;
    delete_recoverable_session_artifacts_with_barrier_while_owned(
        dir,
        &session_id,
        Some(&expected),
        mutation_barrier,
    )
}

pub(super) fn delete_recoverable_session_artifacts_while_owned(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
) -> Result<(), String> {
    delete_recoverable_session_artifacts_with_barrier_while_owned(dir, session_id, expected, || {})
}

pub(super) fn delete_recoverable_session_artifacts_with_barrier_while_owned<F>(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
    mutation_barrier: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    ensure_recoverable_session(dir, session_id)?;
    let intent = build_recoverable_deletion_intent(dir, session_id, expected)?;
    let intent_name = deletion_intent_name(session_id);
    write_deletion_intent_with_publication_barrier_while_owned(
        &dir.join(&intent_name),
        &intent,
        |_| {},
    )?;
    mutation_barrier();
    resume_deletion_intent_while_owned(dir, &intent_name)
}

pub(super) fn build_recoverable_deletion_intent(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
) -> Result<DeletionIntent, String> {
    let mut names = BTreeSet::new();
    for suffix in [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json.part",
        ".commit.json.part",
        ".capture.partial.json",
        ".commit.json",
        ".wav",
        ".txt",
        ".polished.txt",
    ] {
        let name = format!("live-{session_id}{suffix}");
        if regular_artifact_exists(dir, &name) {
            names.insert(name);
        }
    }
    names.extend(transcript_artifact_names(dir, session_id)?);

    let sidecar_name = format!("live-{session_id}.capture.partial.json");
    if names.contains(&sidecar_name)
        && recording::read_regular_artifact(dir, &sidecar_name)
            .is_ok_and(|text| !valid_partial_sidecar(&text, session_id))
    {
        names.remove(&sidecar_name);
    }
    let commit_name = format!("live-{session_id}.commit.json");
    if names.contains(&commit_name) && saved_recovered_session(dir, session_id)?.is_none() {
        names.remove(&commit_name);
    }

    let mut artifacts = names
        .into_iter()
        .map(|name| admit_deletion_artifact(dir, &name))
        .collect::<Result<Vec<_>, _>>()?;
    let anchor_index = if let Some(expected) = expected {
        artifacts
            .iter()
            .position(|artifact| {
                expected.matches_artifact_name(&artifact.name)
                    && artifact.file_identity == Some(expected.file_identity())
            })
            .ok_or_else(|| {
                "admitted recovery artifact is absent from the deletion snapshot".to_string()
            })?
    } else {
        (!artifacts.is_empty())
            .then_some(0)
            .ok_or_else(|| "Recoverable live recording has no deletable artifacts.".to_string())?
    };
    let anchor = artifacts.remove(anchor_index);
    let intent = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: session_id.clone(),
        reason: "recoverable".into(),
        commit_file: anchor.name,
        commit_sha256: anchor.sha256,
        commit_file_identity: anchor.file_identity,
        artifacts,
    };
    validate_deletion_intent(&intent)?;
    Ok(intent)
}

pub(super) fn ensure_recoverable_session(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<(), String> {
    let scan = recording::scan_recordings(dir)?;
    (scan
        .partial
        .into_iter()
        .any(|partial| partial.session_id.as_ref() == Some(session_id))
        || scan
            .recovered_partial
            .into_iter()
            .any(|recovered| recovered.session_id == *session_id))
    .then_some(())
    .ok_or_else(|| "Live recording is not a recoverable Yap session.".into())
}

pub(super) fn saved_recovered_session(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<Option<SavedLiveSession>, String> {
    let commit_name = format!("live-{session_id}.commit.json");
    let Ok((text, _)) = recording::read_and_hash_regular_artifact(dir, &commit_name) else {
        return Ok(None);
    };
    let Ok(commit) = serde_json::from_str::<recording::PartialRecoveryCommit>(&text) else {
        return Ok(None);
    };
    if commit.schema_version != 1
        || commit.status != recording::CaptureStatus::Partial
        || commit.session_id != *session_id
        || !is_expected_recovery_name(&commit.audio_file, session_id, ".wav")
        || !is_expected_recovery_name(
            &commit.capture_sidecar_file,
            session_id,
            ".capture.partial.json",
        )
        || recording::sha256_regular_artifact(dir, &commit.audio_file)? != commit.audio_sha256
        || recording::sha256_regular_artifact(dir, &commit.capture_sidecar_file)?
            != commit.capture_sidecar_sha256
    {
        return Ok(None);
    }
    let audio = recording::open_regular_artifact(dir, &commit.audio_file)?;
    if audio.metadata().map_err(|error| error.to_string())?.len() != commit.audio_bytes {
        return Ok(None);
    }
    let sidecar = recording::read_regular_artifact(dir, &commit.capture_sidecar_file)?;
    if !valid_partial_sidecar(&sidecar, session_id) {
        return Ok(None);
    }
    Ok(Some(SavedLiveSession {
        session_id: session_id.to_string(),
        name: format!("live-{session_id}"),
        source_path: stable_existing_path_string(&dir.join(&commit.audio_file)),
        output_path: stable_existing_path_string(&dir.join(&commit.audio_file)),
        created_at_ms: committed_at_ms(&commit.committed_at_utc),
        warning: Some("Recovered partial recording.".into()),
        capture_commit_path: Some(stable_existing_path_string(&dir.join(commit_name))),
        recovery_state: Some("recoverable".into()),
    }))
}

pub(super) fn valid_partial_sidecar(
    text: &str,
    session_id: &crate::audio::session::SessionId,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
        && value.get("sessionId").and_then(serde_json::Value::as_str) == Some(session_id.as_str())
        && value.get("status").and_then(serde_json::Value::as_str) == Some("partial")
}

pub(super) fn is_expected_recovery_name(
    name: &str,
    session_id: &crate::audio::session::SessionId,
    suffix: &str,
) -> bool {
    recording::validate_artifact_name(name).is_ok() && name == format!("live-{session_id}{suffix}")
}

pub(super) fn artifact_modified_at_ms(dir: &Path, name: &str) -> Option<u64> {
    let file = recording::open_regular_artifact(dir, name).ok()?;
    system_time_to_unix_millis(file.metadata().ok()?.modified().ok()?)
}

pub(super) fn regular_artifact_exists(dir: &std::path::Path, name: &str) -> bool {
    recording::open_regular_artifact(dir, name).is_ok()
}
