use super::*;

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("Failed to hash recording artifact: {error}"))?;
    sha256_open_file(&mut file)
}

pub(super) fn sha256_open_file(file: &mut File) -> Result<String, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("Failed to hash recording artifact: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed to hash recording artifact: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn sha256_open_regular_file(file: &mut File) -> Result<String, String> {
    sha256_open_file(file)
}

pub(super) fn receipt_from_published_sidecar(
    mut file: File,
    file_name: String,
    path: PathBuf,
    expected: &CaptureSidecar,
) -> Result<PublicationReceipt, String> {
    validate_artifact_name(&file_name)?;
    let sha256 = sha256_open_file(&mut file)?;
    let text = read_open_file(&mut file)?;
    let published: CaptureSidecar = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse published capture sidecar: {error}"))?;
    if &published != expected {
        return Err("published capture sidecar does not match the owned sidecar".into());
    }
    Ok(PublicationReceipt {
        file_name,
        sha256,
        status: CaptureStatus::Complete,
        path,
        identity: file_identity(&file)?,
    })
}

pub(super) fn receipt_from_published_partial_sidecar(
    mut file: File,
    file_name: String,
    path: PathBuf,
    expected: &PartialCaptureSidecar,
) -> Result<PublicationReceipt, String> {
    validate_artifact_name(&file_name)?;
    let sha256 = sha256_open_file(&mut file)?;
    let text = read_open_file(&mut file)?;
    let published: PartialCaptureSidecar = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse published partial capture sidecar: {error}"))?;
    if &published != expected {
        return Err("published partial capture sidecar does not match the owned sidecar".into());
    }
    Ok(PublicationReceipt {
        file_name,
        sha256,
        status: CaptureStatus::Partial,
        path,
        identity: file_identity(&file)?,
    })
}

pub(super) fn manifest_from_published_commit(
    file: &mut File,
    expected: &CaptureCommitManifest,
) -> Result<CaptureCommitManifest, String> {
    let _commit_sha256 = sha256_open_file(file)?;
    let text = read_open_file(file)?;
    let published: CaptureCommitManifest = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse published capture commit: {error}"))?;
    published.validate()?;
    if &published != expected {
        return Err("published capture commit does not match the owned commit".into());
    }
    Ok(published)
}

pub(super) fn now_utc() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| "Failed to format recording commit time".to_string())
}

// Windows cannot portably sync a directory handle through std. The false result is persisted so
// callers retain the residual power-loss window instead of mistaking it for durable metadata.
pub(super) fn sync_parent_directory(directory: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let _ = directory;
        false
    }
    #[cfg(not(target_os = "windows"))]
    {
        File::open(directory)
            .and_then(|file| file.sync_all())
            .is_ok()
    }
}

pub(crate) fn sync_recordings_parent(directory: &Path) -> bool {
    sync_parent_directory(directory)
}

pub fn validate_artifact_name(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().count() != 1
        || path.file_name().and_then(|name| name.to_str()) != Some(value)
        || value.contains(':')
    {
        return Err("recording artifact names must be same-directory basenames".into());
    }
    Ok(())
}

pub(super) fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("recording artifact hash is invalid".into())
    }
}

pub(super) fn read_manifest(directory: &Path, name: &str) -> Result<CaptureCommitManifest, String> {
    let text = read_regular_artifact(directory, name)
        .map_err(|error| format!("Failed to read capture commit: {error}"))?;
    let manifest: CaptureCommitManifest = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse capture commit: {error}"))?;
    manifest.validate()?;
    Ok(manifest)
}
