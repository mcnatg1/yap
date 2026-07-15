use super::*;

pub(super) fn classify_model(root: &Path) -> ArtifactInstallState {
    classify_artifacts(root, ARTIFACTS)
}

pub(super) fn classify_artifacts(root: &Path, artifacts: &[Artifact]) -> ArtifactInstallState {
    let mut saw_corrupt = false;

    for artifact in artifacts {
        match classify_artifact(&root.join(artifact.file), artifact) {
            ArtifactInstallState::Missing => return ArtifactInstallState::Missing,
            ArtifactInstallState::Corrupted => saw_corrupt = true,
            ArtifactInstallState::Ready => {}
        }
    }

    if saw_corrupt {
        ArtifactInstallState::Corrupted
    } else {
        ArtifactInstallState::Ready
    }
}

pub(super) fn classify_artifact(path: &Path, artifact: &Artifact) -> ArtifactInstallState {
    if !path.exists() {
        return ArtifactInstallState::Missing;
    }

    match marker_state(path, artifact) {
        MarkerState::Valid => ArtifactInstallState::Ready,
        MarkerState::Missing | MarkerState::Stale => ArtifactInstallState::Corrupted,
    }
}

pub(super) fn marker_state(path: &Path, artifact: &Artifact) -> MarkerState {
    let marker = path.with_extension("verified");
    let Ok(contents) = std::fs::read_to_string(&marker) else {
        return MarkerState::Missing;
    };
    let Ok(metadata) = std::fs::metadata(path) else {
        return MarkerState::Missing;
    };
    let Ok(marker_metadata) = std::fs::metadata(&marker) else {
        return MarkerState::Missing;
    };
    let Ok(artifact_modified) = metadata.modified() else {
        return MarkerState::Stale;
    };
    let Ok(marker_modified) = marker_metadata.modified() else {
        return MarkerState::Stale;
    };

    let mut lines = contents.lines();
    let Some(hash) = lines.next() else {
        return MarkerState::Stale;
    };
    let Some(size) = lines.next().and_then(|size| size.parse::<u64>().ok()) else {
        return MarkerState::Stale;
    };

    if marker_modified < artifact_modified {
        return MarkerState::Stale;
    }

    if hash.eq_ignore_ascii_case(artifact.sha256)
        && size == artifact.bytes
        && metadata.len() == artifact.bytes
    {
        MarkerState::Valid
    } else {
        MarkerState::Stale
    }
}

pub(super) fn verify_or_trust(path: &Path, artifact: &Artifact) -> Result<(), SttError> {
    if marker_state(path, artifact) == MarkerState::Valid {
        return Ok(());
    }
    verify_sha_and_mark(path, artifact)
}

pub(super) fn verify_sha_and_mark(path: &Path, artifact: &Artifact) -> Result<(), SttError> {
    let metadata = std::fs::metadata(path).map_err(|_| SttError::ModelMissing)?;
    if metadata.len() != artifact.bytes {
        return Err(SttError::ModelCorrupt);
    }
    crate::stt::model::verify_sha256(path, artifact.sha256)?;
    write_verified_marker(path, artifact)
}

pub(super) fn write_verified_marker(path: &Path, artifact: &Artifact) -> Result<(), SttError> {
    crate::stt::model::write_text_atomically(
        &path.with_extension("verified"),
        &format!("{}\n{}\n", artifact.sha256, artifact.bytes),
    )
}

pub(super) fn verify_model_at_with_progress<P, C>(
    root: &Path,
    on_progress: &mut P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    P: FnMut(FallbackModelView),
    C: Fn() -> bool + Copy,
{
    verify_artifacts_at_with_progress(root, ARTIFACTS, on_progress, is_cancelled)
}

pub(super) fn verify_artifacts_at_with_progress<P, C>(
    root: &Path,
    artifacts: &[Artifact],
    on_progress: &mut P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    P: FnMut(FallbackModelView),
    C: Fn() -> bool + Copy,
{
    let total_bytes = artifacts.iter().try_fold(0u64, |total, artifact| {
        let size = std::fs::metadata(root.join(artifact.file))
            .map_err(|_| SttError::ModelMissing)?
            .len();
        if size != artifact.bytes {
            return Err(SttError::ModelCorrupt);
        }
        total
            .checked_add(artifact.bytes)
            .ok_or(SttError::ModelCorrupt)
    })?;
    let mut verified_bytes = 0u64;

    for artifact in artifacts {
        if is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
        let path = root.join(artifact.file);
        verify_sha_and_mark(&path, artifact)?;
        verified_bytes = verified_bytes
            .checked_add(artifact.bytes)
            .ok_or(SttError::ModelCorrupt)?;
        on_progress(status_view(
            root,
            FallbackModelStatus::Verifying,
            Some(verified_bytes),
            Some(total_bytes),
            Some(progress_percent(verified_bytes, total_bytes)),
            None,
            Some(format!("Verifying {}", artifact.file)),
        ));
        if is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
    }
    Ok(())
}

fn progress_percent(complete: u64, total: u64) -> f32 {
    if total == 0 {
        return 100.0;
    }
    ((complete as f32 / total as f32) * 100.0).clamp(0.0, 100.0)
}

pub(super) fn status_view(
    root: &Path,
    status: FallbackModelStatus,
    installed_bytes: Option<u64>,
    total_bytes: Option<u64>,
    progress_percent: Option<f32>,
    speed_mbps: Option<f32>,
    message: Option<String>,
) -> FallbackModelView {
    FallbackModelView {
        id: MODEL_ID.to_string(),
        label: FALLBACK_MODEL_LABEL.to_string(),
        status,
        installed_bytes,
        total_bytes,
        progress_percent,
        speed_mbps,
        message,
        models_dir: root.display().to_string(),
    }
}
