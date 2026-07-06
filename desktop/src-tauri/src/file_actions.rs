#[tauri::command]
pub fn read_text_file(window: tauri::WebviewWindow, path: String) -> Result<String, String> {
    ensure_main_window(&window)?;
    read_text_file_at(path)
}

fn read_text_file_at(path: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be read.".into());
    }

    let path = canonical_existing_path(&path)?;
    std::fs::read_to_string(&path).map_err(|err| format!("Failed to read transcript: {err}"))
}

#[tauri::command]
pub fn write_polished_text(
    window: tauri::WebviewWindow,
    path: String,
    text: String,
) -> Result<String, String> {
    ensure_main_window(&window)?;
    write_polished_text_at(path, text)
}

fn write_polished_text_at(path: String, text: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be polished.".into());
    }

    let path = canonical_existing_path(&path)?;
    let output = polished_path(&path)?;
    std::fs::write(&output, text)
        .map_err(|err| format!("Failed to save polished transcript: {err}"))?;
    Ok(output.display().to_string())
}

fn polished_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "Transcript path has no file name.".to_string())?;

    Ok(path.with_file_name(format!("{stem}.polished.txt")))
}

#[tauri::command]
pub fn open_app_path(window: tauri::WebviewWindow, path: String) -> Result<(), String> {
    ensure_main_window(&window)?;
    let path = openable_app_path(path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>)
        .map_err(|err| format!("Failed to open file: {err}"))
}

#[tauri::command]
pub fn reveal_app_path(window: tauri::WebviewWindow, path: String) -> Result<(), String> {
    ensure_main_window(&window)?;
    let path = openable_app_path(path)?;
    tauri_plugin_opener::reveal_item_in_dir(path)
        .map_err(|err| format!("Failed to reveal file: {err}"))
}

#[tauri::command]
pub fn delete_history_entry_files(
    window: tauri::WebviewWindow,
    output_path: String,
    source_path: String,
) -> Result<(), String> {
    ensure_main_window(&window)?;
    delete_history_entry_files_at(output_path, source_path)
}

fn delete_history_entry_files_at(output_path: String, source_path: String) -> Result<(), String> {
    delete_history_entry_files_at_from_dir(output_path, source_path, &crate::live_recordings_dir())
}

fn delete_history_entry_files_at_from_dir(
    output_path: String,
    source_path: String,
    owned_dir: &std::path::Path,
) -> Result<(), String> {
    let output = deletable_transcript_path(output_path)?;
    let source = deletable_yap_owned_recording_path_from_dir(source_path, owned_dir)?;

    if let Some(source) = source.filter(|source| source != &output) {
        std::fs::remove_file(&source)
            .map_err(|err| format!("Failed to delete recording: {err}"))?;
    }

    std::fs::remove_file(&output).map_err(|err| format!("Failed to delete transcript: {err}"))
}

fn openable_app_path(path: String) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    let path = canonical_existing_path(&path)?;
    if !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    Ok(path)
}

fn deletable_transcript_path(path: String) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be deleted.".into());
    }
    let path = canonical_existing_path(&path)?;
    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be deleted.".into());
    }
    Ok(path)
}

fn deletable_yap_owned_recording_path_from_dir(
    path: String,
    owned_dir: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    let path = std::path::PathBuf::from(path);
    if !path.exists() {
        return Ok(None);
    }
    let path = path
        .canonicalize()
        .map_err(|err| format!("Failed to resolve recording path: {err}"))?;
    let Ok(owned_dir) = owned_dir.canonicalize() else {
        return Ok(None);
    };

    if path.starts_with(owned_dir) && is_yap_media_or_transcript_path(&path) {
        Ok(Some(path))
    } else {
        Ok(None)
    }
}

fn canonical_existing_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !path.exists() {
        return Err("File no longer exists.".into());
    }
    path.canonicalize()
        .map_err(|err| format!("Failed to resolve file path: {err}"))
}

pub(crate) fn ensure_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("This file action is only available from the main window.".into())
    }
}

pub(crate) fn is_transcript_path(path: &std::path::Path) -> bool {
    has_extension(path, &["txt"])
}

fn is_yap_media_or_transcript_path(path: &std::path::Path) -> bool {
    has_extension(
        path,
        &["txt", "mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"],
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yap-{name}-{}-{}",
            std::process::id(),
            crate::unix_millis_now().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_text_file_rejects_non_transcripts() {
        assert!(read_text_file_at("recording.mp3".into()).is_err());
    }

    #[test]
    fn app_open_path_allows_only_recordings_and_transcripts() {
        assert!(is_yap_media_or_transcript_path(std::path::Path::new(
            "recording.mp3"
        )));
        assert!(is_yap_media_or_transcript_path(std::path::Path::new(
            "recording.MP4"
        )));
        assert!(is_yap_media_or_transcript_path(std::path::Path::new(
            "recording.txt"
        )));
        assert!(!is_yap_media_or_transcript_path(std::path::Path::new(
            "script.ps1"
        )));
    }

    #[test]
    fn delete_history_entry_files_removes_owned_live_audio() {
        let dir = temp_test_dir("delete-owned-live");
        let transcript = dir.join("live-300.txt");
        let audio = dir.join("live-300.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();

        delete_history_entry_files_at_from_dir(
            transcript.display().to_string(),
            audio.display().to_string(),
            &dir,
        )
        .unwrap();

        assert!(!transcript.exists());
        assert!(!audio.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_history_entry_files_keeps_imported_source_audio() {
        let owned_dir = temp_test_dir("delete-owned-dir");
        let imported_dir = temp_test_dir("delete-imported-source");
        let transcript = imported_dir.join("clip.txt");
        let audio = imported_dir.join("clip.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();

        delete_history_entry_files_at_from_dir(
            transcript.display().to_string(),
            audio.display().to_string(),
            &owned_dir,
        )
        .unwrap();

        assert!(!transcript.exists());
        assert!(audio.exists());
        std::fs::remove_dir_all(owned_dir).ok();
        std::fs::remove_dir_all(imported_dir).ok();
    }

    #[test]
    fn polished_path_writes_sibling_file() {
        let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
        assert_eq!(path.file_name().unwrap(), "take.polished.txt");
    }
}
