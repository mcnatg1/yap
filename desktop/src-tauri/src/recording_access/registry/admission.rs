#[cfg(test)]
use super::MAX_REGISTERED_PLAYBACK_PATHS;
use super::{
    path_policy::{
        canonical_existing_path, canonical_path_is_inside_owned_live_directory,
        is_yap_media_or_transcript_path, playable_recording_path, same_registry_path,
    },
    persistence::read_registered_playback_paths_with_limit,
    MAX_RECORDING_JOB_PLAYBACK_PATHS,
};

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

pub(in crate::recording_access) fn registered_canonical_recording_path_at_with_limit(
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
