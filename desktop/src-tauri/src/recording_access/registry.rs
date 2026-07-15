use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[cfg(test)]
pub(crate) const MAX_REGISTERED_PLAYBACK_PATHS: usize = 500;
pub(super) const MAX_RECORDING_JOB_PLAYBACK_PATHS: usize = 200;
pub(crate) const NATIVE_SELECTION_REGISTRY_VERSION: u32 = 2;

const RECORDING_MEDIA_EXTENSIONS: &[&str] = &["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];

#[derive(Deserialize, Serialize)]
pub(crate) struct RecordingPlaybackRegistry {
    pub(crate) version: u32,
    pub(crate) paths: Vec<String>,
}

pub(crate) fn restore_playback_path_at(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    restore_playback_path_at_with(
        path,
        registry_path,
        owned_dir,
        crate::live::recordings::canonical_committed_live_path_from_dir,
    )
}

pub(crate) fn authorize_openable_app_path(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    openable_app_path_from_registry_paths_with_limits(
        path,
        &[(registry_path, MAX_RECORDING_JOB_PLAYBACK_PATHS)],
        owned_dir,
    )
}

#[cfg(test)]
pub(crate) fn openable_app_path_from_registries(
    path: String,
    general_registry_path: &std::path::Path,
    job_registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    openable_app_path_from_registry_paths_with_limits(
        path,
        &[
            (general_registry_path, MAX_REGISTERED_PLAYBACK_PATHS),
            (job_registry_path, MAX_RECORDING_JOB_PLAYBACK_PATHS),
        ],
        owned_dir,
    )
}

#[cfg(test)]
pub(crate) fn openable_app_path_from(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    openable_app_path_from_registry_paths_with_limits(
        path,
        &[(registry_path, MAX_REGISTERED_PLAYBACK_PATHS)],
        owned_dir,
    )
}

fn openable_app_path_from_registry_paths_with_limits(
    path: String,
    registry_paths: &[(&std::path::Path, usize)],
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    let path = canonical_existing_path(&path)?;
    if !path.is_file() || !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    if canonical_path_is_inside_owned_live_directory(&path, owned_dir) {
        return crate::live::recordings::canonical_committed_live_path_from_dir(
            &path,
            owned_dir,
            crate::live::recordings::is_transcript_path(&path),
        );
    }
    for (registry_path, limit) in registry_paths {
        if registered_canonical_recording_path_at_with_limit(&path, registry_path, *limit).is_ok() {
            return Ok(path);
        }
    }
    Err("Recording file is not registered for playback.".into())
}

#[cfg(test)]
pub(crate) fn register_playback_path_at(
    path: String,
    registry_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    register_playback_path_at_from_owned_dir(
        path,
        registry_path,
        &crate::live::recordings::recordings_dir(),
    )
}

#[cfg(test)]
pub(crate) fn register_playback_path_at_from_owned_dir(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    register_playback_path_at_from_owned_dir_with_limit(
        path,
        registry_path,
        owned_dir,
        MAX_REGISTERED_PLAYBACK_PATHS,
        "The playback registry is full; remove an old imported recording before adding another.",
    )
}

#[cfg(test)]
pub(crate) fn register_general_playback_path_at_for_test(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    register_playback_path_at_from_owned_dir(path, registry_path, owned_dir)
}

pub(crate) fn register_recording_job_playback_path_at_from_owned_dir(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    register_playback_path_at_from_owned_dir_with_limit(
        path,
        registry_path,
        owned_dir,
        MAX_RECORDING_JOB_PLAYBACK_PATHS,
        "The recording job playback registry is full.",
    )
}

fn register_playback_path_at_from_owned_dir_with_limit(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
    limit: usize,
    full_message: &str,
) -> Result<std::path::PathBuf, String> {
    let path = playable_recording_path(path)?;
    if canonical_path_is_inside_owned_live_directory(&path, owned_dir) {
        return crate::live::recordings::canonical_committed_live_path_from_dir(
            &path, owned_dir, false,
        );
    }
    let _guard = playback_registry_lock()
        .lock()
        .map_err(|_| "Playback registry lock is unavailable.".to_string())?;
    let mut paths = read_registered_playback_paths_with_limit(registry_path, limit)?;
    let already_registered = paths
        .iter()
        .any(|registered| same_registry_path(registered, &path));
    if !already_registered && paths.len() >= limit {
        return Err(full_message.into());
    }
    paths.retain(|registered| !same_registry_path(registered, &path));
    paths.insert(0, path.clone());
    write_registered_playback_paths(registry_path, &paths)?;
    Ok(path)
}

fn playback_registry_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
pub(crate) fn registered_playback_path_at(
    path: String,
    registry_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let path = playable_recording_path(path)?;
    registered_canonical_recording_path_at(&path, registry_path)
}

pub(crate) fn restore_playback_path_at_with<F>(
    path: String,
    registry_path: &std::path::Path,
    owned_dir: &std::path::Path,
    authorize_owned: F,
) -> Result<std::path::PathBuf, String>
where
    F: FnOnce(&std::path::Path, &std::path::Path, bool) -> Result<std::path::PathBuf, String>,
{
    let path = playable_recording_path(path)?;
    if canonical_path_is_inside_owned_live_directory(&path, owned_dir) {
        return authorize_owned(&path, owned_dir, false);
    }
    registered_canonical_recording_path_at_with_limit(
        &path,
        registry_path,
        MAX_RECORDING_JOB_PLAYBACK_PATHS,
    )
}

#[cfg(test)]
fn registered_canonical_recording_path_at(
    path: &std::path::Path,
    registry_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    registered_canonical_recording_path_at_with_limit(
        path,
        registry_path,
        MAX_REGISTERED_PLAYBACK_PATHS,
    )
}

pub(super) fn registered_canonical_recording_path_at_with_limit(
    path: &std::path::Path,
    registry_path: &std::path::Path,
    limit: usize,
) -> Result<std::path::PathBuf, String> {
    if read_registered_playback_paths_with_limit(registry_path, limit)?
        .iter()
        .any(|registered| same_registry_path(registered, path))
    {
        return Ok(path.to_path_buf());
    }
    Err("Recording file is not registered for playback.".into())
}

pub(crate) fn recording_job_playback_registry_path() -> std::path::PathBuf {
    crate::paths::app_data_dir().join("recording-job-playback-registry.json")
}

pub(crate) fn recording_job_selection_registry_path() -> std::path::PathBuf {
    crate::paths::app_data_dir().join("recording-native-selection-registry.json")
}

#[cfg(test)]
pub(crate) fn read_registered_playback_paths(
    registry_path: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    read_registered_playback_paths_with_limit(registry_path, MAX_REGISTERED_PLAYBACK_PATHS)
}

fn read_registered_playback_paths_with_limit(
    registry_path: &std::path::Path,
    limit: usize,
) -> Result<Vec<std::path::PathBuf>, String> {
    let text = match std::fs::read_to_string(registry_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Failed to read playback registry: {error}")),
    };
    let Ok(registry) = serde_json::from_str::<RecordingPlaybackRegistry>(&text) else {
        return Ok(Vec::new());
    };
    if registry.version == 1 {
        return Ok(Vec::new());
    }
    if registry.version != NATIVE_SELECTION_REGISTRY_VERSION {
        return Err(format!(
            "Unsupported playback registry version {}.",
            registry.version
        ));
    }

    Ok(registry
        .paths
        .into_iter()
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute() && is_recording_media_path(path))
        .take(limit)
        .collect())
}

pub(crate) fn remove_recording_job_playback_path_at(
    path: &std::path::Path,
    registry_path: &std::path::Path,
) -> Result<(), String> {
    let _guard = playback_registry_lock()
        .lock()
        .map_err(|_| "Playback registry lock is unavailable.".to_string())?;
    let mut paths =
        read_registered_playback_paths_with_limit(registry_path, MAX_RECORDING_JOB_PLAYBACK_PATHS)?;
    let original_len = paths.len();
    paths.retain(|registered| !same_registry_path(registered, path));
    if paths.len() != original_len {
        write_registered_playback_paths(registry_path, &paths)?;
    }
    Ok(())
}

pub(crate) fn reconcile_recording_job_playback_paths_at(
    recoverable_paths: &[std::path::PathBuf],
    registry_path: &std::path::Path,
) -> Result<(), String> {
    let _guard = playback_registry_lock()
        .lock()
        .map_err(|_| "Playback registry lock is unavailable.".to_string())?;
    let registered =
        read_registered_playback_paths_with_limit(registry_path, MAX_RECORDING_JOB_PLAYBACK_PATHS)?;
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for path in recoverable_paths {
        if !path.is_absolute() || !is_recording_media_path(path) {
            continue;
        }
        if !registered
            .iter()
            .any(|registered| same_registry_path(registered, path))
        {
            continue;
        }
        if paths
            .iter()
            .any(|registered| same_registry_path(registered, path))
        {
            continue;
        }
        paths.push(path.clone());
        if paths.len() == MAX_RECORDING_JOB_PLAYBACK_PATHS {
            break;
        }
    }
    write_registered_playback_paths(registry_path, &paths)
}

pub(crate) fn write_registered_playback_paths(
    registry_path: &std::path::Path,
    paths: &[std::path::PathBuf],
) -> Result<(), String> {
    if let Some(parent) = registry_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to prepare playback registry: {err}"))?;
    }

    let registry = RecordingPlaybackRegistry {
        version: NATIVE_SELECTION_REGISTRY_VERSION,
        paths: paths
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
    };
    let text = serde_json::to_string_pretty(&registry)
        .map_err(|err| format!("Failed to serialize playback registry: {err}"))?;
    crate::atomic_text::write(registry_path, &text)
        .map_err(|err| format!("Failed to save playback registry: {err}"))
}

pub(crate) fn playable_recording_path(path: String) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_recording_media_path(&path) {
        return Err("Choose a supported audio or video file.".into());
    }
    let path = canonical_existing_path(&path)?;
    if !path.is_file() || !is_recording_media_path(&path) {
        return Err("Choose a supported audio or video file.".into());
    }
    Ok(path)
}

fn canonical_existing_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !path.exists() {
        return Err("File no longer exists.".into());
    }
    path.canonicalize()
        .map_err(|err| format!("Failed to resolve file path: {err}"))
}

pub(super) fn canonical_path_is_inside_owned_live_directory(
    path: &std::path::Path,
    owned_dir: &std::path::Path,
) -> bool {
    owned_dir
        .canonicalize()
        .is_ok_and(|owned| path.starts_with(owned))
}

pub(crate) fn is_yap_media_or_transcript_path(path: &std::path::Path) -> bool {
    crate::live::recordings::is_transcript_path(path) || is_recording_media_path(path)
}

fn is_recording_media_path(path: &std::path::Path) -> bool {
    has_extension(path, RECORDING_MEDIA_EXTENSIONS)
}

fn has_extension(path: &std::path::Path, allowed: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            allowed
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn same_registry_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    if cfg!(windows) {
        return left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy());
    }
    left == right
}
