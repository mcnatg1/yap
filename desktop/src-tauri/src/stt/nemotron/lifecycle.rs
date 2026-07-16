use super::{
    catalog::{classify_artifacts, status_view, verify_or_trust, write_verified_marker},
    *,
};

pub(super) fn paths_at(root: PathBuf) -> Result<NemotronPaths, SttError> {
    Ok(NemotronPaths {
        encoder: require(root.join("encoder.int8.onnx"))?,
        decoder: require(root.join("decoder.int8.onnx"))?,
        joiner: require(root.join("joiner.int8.onnx"))?,
        tokens: require(root.join("tokens.txt"))?,
    })
}

fn require(path: PathBuf) -> Result<PathBuf, SttError> {
    path.exists().then_some(path).ok_or(SttError::ModelMissing)
}

pub(super) fn ensure_artifact<P>(
    root: &Path,
    artifact: &Artifact,
    force: bool,
    on_progress: &mut P,
    operation: &crate::stt::model::DownloadOperation,
) -> Result<(), SttError>
where
    P: FnMut(FallbackModelView),
{
    let dest = root.join(artifact.file);
    crate::stt::model::cleanup_stale_download_temps(&dest, operation)?;

    if !force && verify_or_trust(&dest, artifact).is_ok() {
        return Ok(());
    }

    let url = crate::stt::model::hf_resolve_url(REPO, REVISION, artifact.file);
    crate::stt::model::download_verified_file(
        &crate::stt::model::DownloadRequest {
            url,
            destination: dest.clone(),
            expected_bytes: artifact.bytes,
            expected_sha256: artifact.sha256.to_string(),
        },
        operation,
        |progress| {
            on_progress(status_view(
                root,
                FallbackModelStatus::Downloading,
                Some(progress.downloaded_bytes),
                progress.total_bytes,
                progress.percent(),
                progress.speed_mbps(),
                Some(format!("Downloading {}", artifact.file)),
            ));
        },
    )?;

    on_progress(status_view(
        root,
        FallbackModelStatus::Verifying,
        Some(artifact.bytes),
        Some(artifact.bytes),
        Some(100.0),
        None,
        Some(format!("Verifying {}", artifact.file)),
    ));

    write_verified_marker(&dest, artifact)?;
    if operation.is_cancelled() {
        return Err(SttError::ModelInstallCancelled);
    }

    Ok(())
}

pub(super) fn resolve_model_at(root: &Path) -> Result<NemotronPaths, SttError> {
    resolve_model_at_with_artifacts(root, ARTIFACTS)
}

pub(super) fn resolve_model_at_with_artifacts(
    root: &Path,
    artifacts: &[Artifact],
) -> Result<NemotronPaths, SttError> {
    match classify_artifacts(root, artifacts) {
        ArtifactInstallState::Missing => Err(SttError::ModelMissing),
        ArtifactInstallState::Corrupted => Err(SttError::ModelCorrupt),
        ArtifactInstallState::Ready => paths_at(root.to_path_buf()),
    }
}

pub(super) fn local_fallback_start_paths_at(
    root: &Path,
    enabled: bool,
) -> Result<NemotronPaths, SttError> {
    local_fallback_start_paths_at_with_artifacts(root, enabled, ARTIFACTS)
}

pub(super) fn local_fallback_start_paths_at_with_artifacts(
    root: &Path,
    enabled: bool,
    artifacts: &[Artifact],
) -> Result<NemotronPaths, SttError> {
    if !enabled {
        return Err(SttError::FallbackDisabled);
    }
    resolve_model_at_with_artifacts(root, artifacts)
}

pub(super) fn load_local_fallback_at_with_artifacts<T, F>(
    root: &Path,
    enabled: bool,
    artifacts: &[Artifact],
    loader: F,
) -> Result<LoadedNemotronModel<T>, SttError>
where
    F: FnOnce(&NemotronPaths) -> Result<T, SttError>,
{
    if !enabled {
        return Err(SttError::FallbackDisabled);
    }
    match classify_artifacts(root, artifacts) {
        ArtifactInstallState::Missing => return Err(SttError::ModelMissing),
        ArtifactInstallState::Corrupted => return Err(SttError::ModelCorrupt),
        ArtifactInstallState::Ready => {}
    }
    let guard = ModelLoadGuard::open(root, artifacts)?;
    let value = loader(guard.paths())?;
    guard.revalidate_after_native_load()?;
    Ok(LoadedNemotronModel { value, guard })
}

fn remove_if_exists(path: PathBuf) -> Result<(), SttError> {
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() => return Err(SttError::ModelCorrupt),
        Ok(_) => {
            std::fs::remove_file(path).map_err(|_| SttError::ModelMissing)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(SttError::ModelMissing),
    }
    Ok(())
}

pub(super) fn remove_download_artifacts(path: &Path) -> Result<(), SttError> {
    remove_if_exists(path.to_path_buf())?;
    remove_if_exists(path.with_extension("verified"))?;
    remove_if_exists(path.with_extension("part"))?;
    remove_unique_partial_artifacts(path)?;
    Ok(())
}

fn remove_unique_partial_artifacts(path: &Path) -> Result<(), SttError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if !parent.exists() {
        return Ok(());
    }
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let prefix = format!("{file_name}.");

    let entries = std::fs::read_dir(parent).map_err(|_| SttError::ModelMissing)?;
    for entry in entries {
        let entry = entry.map_err(|_| SttError::ModelMissing)?;
        let candidate = entry.path();
        let Some(candidate_name) = candidate.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if candidate_name.starts_with(&prefix) && candidate_name.ends_with(".part") {
            remove_if_exists(candidate)?;
        }
    }
    Ok(())
}
