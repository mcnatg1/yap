const RECORDING_MEDIA_EXTENSIONS: &[&str] = &["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];

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

pub(super) fn canonical_existing_path(
    path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    if !path.exists() {
        return Err("File no longer exists.".into());
    }
    path.canonicalize()
        .map_err(|err| format!("Failed to resolve file path: {err}"))
}

pub(in crate::recording_access) fn canonical_path_is_inside_owned_live_directory(
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

pub(super) fn is_recording_media_path(path: &std::path::Path) -> bool {
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

pub(super) fn same_registry_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    if cfg!(windows) {
        return left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy());
    }
    left == right
}
