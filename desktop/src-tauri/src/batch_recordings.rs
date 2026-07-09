use std::path::{Path, PathBuf};

const MAX_TRANSCRIBE_BATCH_PATHS: usize = 200;
const RECORDING_EXTENSIONS: &[&str] = &["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];

pub(crate) fn validate_recording_paths(
    paths: &[String],
) -> Result<Vec<PathBuf>, crate::stt::dispatch::SttCommandError> {
    if paths.len() > MAX_TRANSCRIBE_BATCH_PATHS {
        return Err(invalid_recording_path_error(
            "Too many recordings queued. Add fewer files at once.",
        ));
    }

    paths
        .iter()
        .map(|path| validate_recording_path(path))
        .collect()
}

fn validate_recording_path(path: &str) -> Result<PathBuf, crate::stt::dispatch::SttCommandError> {
    let path = PathBuf::from(path);
    if !is_supported_recording_path(&path) {
        return Err(invalid_recording_path_error(
            "Choose a supported audio or video file.",
        ));
    }
    let path = path
        .canonicalize()
        .map_err(|_| invalid_recording_path_error("Recording file no longer exists."))?;
    if !path.is_file() || !is_supported_recording_path(&path) {
        return Err(invalid_recording_path_error(
            "Choose a supported audio or video file.",
        ));
    }
    Ok(path)
}

pub(crate) fn is_supported_recording_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            RECORDING_EXTENSIONS
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        })
}

fn invalid_recording_path_error(message: &str) -> crate::stt::dispatch::SttCommandError {
    crate::stt::dispatch::SttCommandError {
        code: crate::stt::error::SttError::AudioDecode.code().to_string(),
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yap-batch-recordings-{name}-{}-{}",
            std::process::id(),
            crate::live::recordings::unix_millis_now().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recording_path_validation_accepts_existing_media_files() {
        let dir = temp_test_dir("recording-path-ok");
        let recording = dir.join("meeting.WAV");
        std::fs::write(&recording, b"RIFF").unwrap();

        let validated = validate_recording_paths(&[recording.display().to_string()]).unwrap();

        assert_eq!(validated.len(), 1);
        assert!(validated[0].is_absolute());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recording_path_validation_rejects_fake_media_directories() {
        let dir = temp_test_dir("recording-path-dir");
        let recording_dir = dir.join("meeting.wav");
        std::fs::create_dir_all(&recording_dir).unwrap();

        let error = validate_recording_paths(&[recording_dir.display().to_string()]).unwrap_err();

        assert_eq!(error.code, crate::stt::error::SttError::AudioDecode.code());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recording_path_validation_rejects_resolved_non_media_targets() {
        let dir = temp_test_dir("recording-path-symlink");
        let target = dir.join("script.ps1");
        let link = dir.join("meeting.wav");
        std::fs::write(&target, "Write-Host nope").unwrap();
        if create_file_symlink(&target, &link).is_err() {
            std::fs::remove_dir_all(dir).ok();
            return;
        }

        let error = validate_recording_paths(&[link.display().to_string()]).unwrap_err();

        assert_eq!(error.code, crate::stt::error::SttError::AudioDecode.code());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recording_path_validation_bounds_batch_size() {
        let paths = (0..=MAX_TRANSCRIBE_BATCH_PATHS)
            .map(|index| format!("C:/recording-{index}.wav"))
            .collect::<Vec<_>>();

        let error = validate_recording_paths(&paths).unwrap_err();

        assert_eq!(
            error.message,
            "Too many recordings queued. Add fewer files at once."
        );
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }
}
