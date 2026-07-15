use super::super::deletion::admit_expected_private_artifact_identity;
use super::super::*;
use super::{
    catalog::{
        recoverable_session_artifact_path, recoverable_session_from_dir,
        saved_session_action_artifact_path,
    },
    deletion::ensure_recoverable_session,
};

pub(in crate::live::recordings) fn recover_live_session_in_dir(
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

pub(in crate::live::recordings) fn recover_live_session_in_dir_with_mutation_barrier<F>(
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

pub(in crate::live::recordings) fn recover_live_session_in_dir_with_queue_observer<F, Q>(
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

pub(in crate::live::recordings) fn saved_recovered_session(
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

pub(in crate::live::recordings) fn valid_partial_sidecar(
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

pub(in crate::live::recordings) fn is_expected_recovery_name(
    name: &str,
    session_id: &crate::audio::session::SessionId,
    suffix: &str,
) -> bool {
    recording::validate_artifact_name(name).is_ok() && name == format!("live-{session_id}{suffix}")
}
