use super::*;

pub fn scan_recordings(directory: &Path) -> Result<RecordingScan, String> {
    if !directory.exists() {
        return Ok(RecordingScan::default());
    }
    let mut scan = RecordingScan::default();
    let mut partial_ids = BTreeSet::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Failed to read live recordings: {error}"))?
    {
        let entry = entry.map_err(|error| format!("Failed to read live recording: {error}"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_regular_artifact(&directory.join(name)) {
            continue;
        }
        if let Some(session) = session_from_private_artifact(name) {
            if name.ends_with(".capture.journal.part") {
                let _ = read_journal_append_log(directory, name);
            }
            partial_ids.insert(session.as_str().to_string());
            continue;
        }
        if let Some(session) = session_from_orphan_wav_artifact(name) {
            if has_owned_partial_lineage(directory, &session) {
                partial_ids.insert(session.as_str().to_string());
            }
            continue;
        }
        let Some(session) = session_from_commit_artifact(name) else {
            continue;
        };
        match read_manifest(directory, name)
            .and_then(|manifest| validate_committed_capture(directory, manifest))
        {
            Ok(committed) => scan.complete.push(committed),
            Err(complete_error) => {
                match read_recovered_partial_capture(directory, name, &session) {
                    Ok(()) => scan.recovered_partial.push(RecoveredPartialCapture {
                        session_id: session,
                        directory: directory.to_path_buf(),
                    }),
                    Err(_) => scan.damaged.push(DamagedCommittedCapture {
                        session_id: session,
                        directory: directory.to_path_buf(),
                        reason: bounded_scan_reason(&complete_error),
                    }),
                }
            }
        }
    }
    for session in partial_ids {
        let session_id =
            SessionId::new(session).expect("private artifact parser validates session IDs");
        if !scan
            .complete
            .iter()
            .any(|capture| capture.manifest.session_id == session_id)
            && !scan
                .recovered_partial
                .iter()
                .any(|capture| capture.session_id == session_id)
            && !scan
                .damaged
                .iter()
                .any(|capture| capture.session_id == session_id)
        {
            scan.partial.push(PartialCapture {
                session_id: Some(session_id),
                directory: directory.to_path_buf(),
            });
        }
    }
    Ok(scan)
}

fn session_from_commit_artifact(name: &str) -> Option<SessionId> {
    name.strip_prefix("live-")
        .and_then(|value| value.strip_suffix(".commit.json"))
        .and_then(|session| SessionId::new(session.to_string()).ok())
}

fn bounded_scan_reason(error: &str) -> String {
    let detail = error.chars().take(160).collect::<String>();
    format!("Damaged complete capture commit: {detail}")
}

fn read_recovered_partial_capture(
    directory: &Path,
    name: &str,
    expected_session: &SessionId,
) -> Result<(), String> {
    let text = read_regular_artifact(directory, name)
        .map_err(|error| format!("Failed to read partial recovery commit: {error}"))?;
    let commit: PartialRecoveryCommit = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse partial recovery commit: {error}"))?;
    commit.validate()?;
    if &commit.session_id != expected_session {
        return Err("partial recovery commit session does not match its file name".into());
    }
    let mut audio = open_regular_artifact(directory, &commit.audio_file)?;
    let mut sidecar = open_regular_artifact(directory, &commit.capture_sidecar_file)?;
    if audio
        .metadata()
        .map_err(|error| format!("Failed to inspect recovered partial audio: {error}"))?
        .len()
        != commit.audio_bytes
        || sha256_open_file(&mut audio)? != commit.audio_sha256
        || sha256_open_file(&mut sidecar)? != commit.capture_sidecar_sha256
    {
        return Err("partial recovery artifact hash does not match the commit".into());
    }
    let sidecar = read_open_file(&mut sidecar)
        .map_err(|error| format!("Failed to read partial recovery sidecar: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&sidecar)
        .map_err(|error| format!("Failed to parse partial recovery sidecar: {error}"))?;
    if value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
        || value.get("sessionId").and_then(serde_json::Value::as_str)
            != Some(expected_session.as_str())
        || value.get("status").and_then(serde_json::Value::as_str) != Some("partial")
    {
        return Err("partial recovery sidecar does not match the commit".into());
    }
    Ok(())
}

fn validate_committed_capture(
    directory: &Path,
    manifest: CaptureCommitManifest,
) -> Result<CommittedCapture, String> {
    manifest.validate()?;
    let mut audio = open_regular_artifact(directory, &manifest.audio_file)?;
    let mut sidecar = open_regular_artifact(directory, &manifest.capture_sidecar_file)?;
    if audio
        .metadata()
        .map_err(|error| format!("Failed to inspect committed audio: {error}"))?
        .len()
        != manifest.audio_bytes
    {
        return Err("committed audio size does not match the manifest".into());
    }
    if sha256_open_file(&mut audio)? != manifest.audio_sha256
        || sha256_open_file(&mut sidecar)? != manifest.capture_sidecar_sha256
    {
        return Err("committed recording artifact hash does not match the manifest".into());
    }
    let sidecar_text = read_open_file(&mut sidecar)
        .map_err(|error| format!("Failed to read capture sidecar: {error}"))?;
    let sidecar: CaptureSidecar = serde_json::from_str(&sidecar_text)
        .map_err(|error| format!("Failed to parse capture sidecar: {error}"))?;
    sidecar.validate(&manifest)?;
    Ok(CommittedCapture {
        manifest,
        directory: directory.to_path_buf(),
    })
}

pub(crate) fn is_regular_artifact(path: &Path) -> bool {
    let Some(directory) = path.parent() else {
        return false;
    };
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    open_regular_artifact(directory, name).is_ok()
}

pub(crate) fn open_regular_artifact(directory: &Path, name: &str) -> Result<File, String> {
    validate_artifact_name(name)?;
    let path = directory.join(name);
    let file = open_no_follow(&path)
        .map_err(|error| format!("Failed to open recording artifact: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording artifact: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("recording artifact is not a regular file".into());
    }
    #[cfg(windows)]
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("recording artifact is a reparse point".into());
    }
    Ok(file)
}

pub(crate) fn recover_partial_wav_with_identity(
    directory: &Path,
    session_id: &SessionId,
    expected: &RegularArtifactIdentity,
) -> Result<(String, u64, String), String> {
    recover_partial_wav_with_admitted_identity(directory, session_id, Some(expected))
}

fn recover_partial_wav_with_admitted_identity(
    directory: &Path,
    session_id: &SessionId,
    expected: Option<&RegularArtifactIdentity>,
) -> Result<(String, u64, String), String> {
    let source_name = format!("live-{session_id}.wav.part");
    let destination_name = format!("live-{session_id}.wav");
    let source = directory.join(&source_name);
    let destination = directory.join(&destination_name);
    if open_regular_artifact(directory, &source_name).is_err() {
        let mut orphan = open_regular_artifact(directory, &destination_name)?;
        if let Some(expected) = expected {
            if !expected.matches_artifact_name(&destination_name) {
                return Err(
                    "admitted recovery artifact no longer matches the current session".into(),
                );
            }
            expected.ensure_open_file(&orphan)?;
        }
        validate_recoverable_wav(&mut orphan, "recovered live audio")?;
        let bytes = orphan
            .metadata()
            .map_err(|error| format!("Failed to inspect recovered live audio: {error}"))?
            .len();
        let hash = sha256_open_file(&mut orphan)?;
        return Ok((destination_name, bytes, hash));
    }

    let mut audio = open_regular_artifact_for_update(directory, &source_name)?;
    if let Some(expected) = expected {
        if !expected.matches_artifact_name(&source_name) {
            return Err("admitted recovery artifact no longer matches the current session".into());
        }
        expected.ensure_open_file(&audio)?;
    }
    let (data_bytes, _) = validate_recoverable_wav(&mut audio, "partial live audio")?;
    write_wav_header(&mut audio, data_bytes)?;
    audio
        .sync_all()
        .map_err(|error| format!("Failed to sync recovered live audio: {error}"))?;
    let mut published = publish_no_replace(
        &source,
        &destination,
        &audio,
        "publish recovered live audio",
    )?;
    let published_bytes = published
        .metadata()
        .map_err(|error| format!("Failed to inspect recovered live audio: {error}"))?
        .len();
    let hash = sha256_open_file(&mut published)?;
    Ok((destination_name, published_bytes, hash))
}
