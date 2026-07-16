use super::*;

pub(crate) fn admit_expected_regular_artifact(
    actual_path: &Path,
    expected_path: &Path,
) -> Result<RegularArtifactIdentity, String> {
    admit_expected_regular_artifact_with_link_policy(actual_path, expected_path, false)
}

pub(crate) fn admit_regular_artifact(path: &Path) -> Result<RegularArtifactIdentity, String> {
    let file = open_regular_path(path)?;
    Ok(RegularArtifactIdentity {
        path: path.to_path_buf(),
        identity: file_identity(&file)?,
        require_single_link: false,
    })
}

pub(crate) fn admit_expected_private_regular_artifact(
    actual_path: &Path,
    expected_path: &Path,
) -> Result<RegularArtifactIdentity, String> {
    admit_expected_regular_artifact_with_link_policy(actual_path, expected_path, true)
}

fn admit_expected_regular_artifact_with_link_policy(
    actual_path: &Path,
    expected_path: &Path,
    require_single_link: bool,
) -> Result<RegularArtifactIdentity, String> {
    let actual = open_regular_path(actual_path)?;
    let expected = open_regular_path(expected_path)?;
    if !same_file_identity(&actual, &expected)? {
        return Err(
            "Live recording identity is no longer current. Refresh history and try again.".into(),
        );
    }
    let admitted = RegularArtifactIdentity {
        path: actual_path.to_path_buf(),
        identity: file_identity(&actual)?,
        require_single_link,
    };
    admitted.ensure_link_ownership(&actual)?;
    Ok(admitted)
}

pub(crate) fn remove_regular_artifact(directory: &Path, name: &str) -> Result<(), String> {
    let owned = open_regular_artifact(directory, name)?;
    remove_open_regular_artifact(directory, name, &owned, || {})
}

pub(crate) fn quarantine_regular_artifact(
    directory: &Path,
    name: &str,
) -> Result<QuarantinedArtifact, String> {
    let mut owned = open_regular_artifact(directory, name)?;
    let sha256 = sha256_open_file(&mut owned)?;
    let identity = file_identity(&owned)?;
    let path = quarantine_open_regular_artifact(directory, name, &owned)?;
    Ok(QuarantinedArtifact {
        path,
        sha256,
        identity,
    })
}

pub(crate) fn verified_regular_artifact(
    directory: &Path,
    name: &str,
) -> Result<QuarantinedArtifact, String> {
    let mut owned = open_regular_artifact(directory, name)?;
    let sha256 = sha256_open_file(&mut owned)?;
    let identity = file_identity(&owned)?;
    Ok(QuarantinedArtifact {
        path: directory.join(name),
        sha256,
        identity,
    })
}

pub(crate) fn remove_verified_quarantined_artifact(
    artifact: &QuarantinedArtifact,
) -> Result<(), String> {
    let mut current = open_regular_path(&artifact.path)?;
    if file_identity(&current)? != artifact.identity
        || sha256_open_file(&mut current)? != artifact.sha256
    {
        return Err(
            "quarantined recording artifact no longer matches its verified identity or hash".into(),
        );
    }
    let directory = artifact
        .path
        .parent()
        .ok_or_else(|| "quarantined recording artifact has no parent directory".to_string())?;
    let name = artifact
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "quarantined recording artifact has no valid file name".to_string())?;
    remove_open_regular_artifact(directory, name, &current, || {})
}

pub(crate) fn restore_verified_quarantined_artifact(
    artifact: &QuarantinedArtifact,
    destination: &Path,
) -> Result<(), String> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "recording artifact has no valid restore destination".to_string())?;
    validate_artifact_name(name)?;
    if destination.exists() {
        return Err("recording artifact restore destination is already occupied".into());
    }
    let mut current = open_regular_path(&artifact.path)?;
    if file_identity(&current)? != artifact.identity
        || sha256_open_file(&mut current)? != artifact.sha256
    {
        return Err(
            "quarantined recording artifact no longer matches its verified identity or hash".into(),
        );
    }
    fs::hard_link(&artifact.path, destination)
        .map_err(|error| format!("Failed to restore quarantined recording artifact: {error}"))?;
    let restored = open_regular_path(destination)?;
    if file_identity(&restored)? != artifact.identity {
        let _ = fs::remove_file(destination);
        return Err("restored recording artifact no longer matches its verified identity".into());
    }
    drop(restored);
    drop(current);
    remove_verified_quarantined_artifact(artifact)
}

#[cfg(test)]
pub(crate) fn remove_regular_artifact_if_hash(
    directory: &Path,
    name: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut owned = open_regular_artifact(directory, name)?;
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    remove_open_regular_artifact(directory, name, &owned, || {})
}

pub(crate) fn revalidate_regular_artifact_identity(
    expected: &RegularArtifactIdentity,
) -> Result<(), String> {
    expected.open_current().map(drop)
}

pub(crate) fn remove_regular_artifact_if_identity_and_hash(
    directory: &Path,
    name: &str,
    expected: &RegularArtifactIdentity,
    expected_sha256: &str,
) -> Result<(), String> {
    let path = directory.join(name);
    if !expected.matches_artifact_name(name) {
        return Err("admitted recording artifact no longer matches the deletion target".into());
    }
    let mut owned = expected.open_current_at(&path)?;
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    remove_open_regular_artifact(directory, name, &owned, || {})
}

pub(crate) fn revalidate_regular_artifact_file_identity_and_hash(
    directory: &Path,
    name: &str,
    expected: &FileIdentity,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut owned = open_regular_artifact(directory, name)?;
    if file_identity(&owned)? != *expected {
        return Err("recording artifact no longer matches its admitted identity".into());
    }
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    Ok(())
}

pub(crate) fn remove_regular_artifact_if_file_identity_and_hash(
    directory: &Path,
    name: &str,
    expected: &FileIdentity,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut owned = open_regular_artifact(directory, name)?;
    if file_identity(&owned)? != *expected {
        return Err("recording artifact no longer matches its admitted identity".into());
    }
    if sha256_open_file(&mut owned)? != expected_sha256 {
        return Err("recording artifact no longer matches its validated hash".into());
    }
    remove_open_regular_artifact(directory, name, &owned, || {})
}

#[cfg(test)]
pub(super) fn remove_regular_artifact_with_barrier_for_test<F>(
    directory: &Path,
    name: &str,
    barrier: F,
) -> Result<(), String>
where
    F: FnOnce(&Path),
{
    let owned = open_regular_artifact(directory, name)?;
    remove_open_regular_artifact(directory, name, &owned, || barrier(&directory.join(name)))
}

pub(super) fn remove_open_regular_artifact<F>(
    directory: &Path,
    name: &str,
    owned: &File,
    before_quarantine: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    before_quarantine();
    let quarantine = quarantine_open_regular_artifact(directory, name, owned)?;
    fs::remove_file(&quarantine)
        .map_err(|error| format!("Failed to remove quarantined recording artifact: {error}"))
}

fn quarantine_open_regular_artifact(
    directory: &Path,
    name: &str,
    owned: &File,
) -> Result<PathBuf, String> {
    validate_artifact_name(name)?;
    let path = directory.join(name);
    let quarantine = unique_delete_quarantine_path(directory, name)?;
    fs::rename(&path, &quarantine).map_err(|error| {
        format!("Failed to quarantine recording artifact for deletion: {error}")
    })?;
    let quarantined = match open_regular_path(&quarantine) {
        Ok(file) => file,
        Err(error) => {
            return Err(format!(
                "Failed to verify quarantined recording artifact: {error}"
            ))
        }
    };
    if !same_file_identity(owned, &quarantined)? {
        let _ = restore_quarantined_artifact(&quarantine, &path);
        return Err("recording artifact path no longer names the verified file".into());
    }
    drop(quarantined);
    Ok(quarantine)
}

fn unique_delete_quarantine_path(directory: &Path, name: &str) -> Result<PathBuf, String> {
    for _ in 0..128 {
        let nonce = DELETE_QUARANTINE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let candidate = directory.join(format!(".{name}.delete-{}-{nonce}", std::process::id()));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("Failed to allocate a private recording deletion quarantine path".into())
}

fn restore_quarantined_artifact(quarantine: &Path, path: &Path) -> Result<(), String> {
    std::fs::hard_link(quarantine, path)
        .map_err(|error| format!("Failed to restore quarantined recording artifact: {error}"))?;
    std::fs::remove_file(quarantine).map_err(|error| {
        format!("Failed to finish restoring quarantined recording artifact: {error}")
    })
}
