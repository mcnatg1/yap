use std::io::{Read, Write};

const MAX_TRANSCRIPT_READ_BYTES: u64 = 2 * 1024 * 1024;

#[tauri::command]
pub fn read_text_file(window: tauri::WebviewWindow, path: String) -> Result<String, String> {
    ensure_main_window(&window)?;
    read_text_file_at(path)
}

#[tauri::command]
pub fn read_text_preview(
    window: tauri::WebviewWindow,
    path: String,
    max_chars: Option<usize>,
) -> Result<String, String> {
    ensure_main_window(&window)?;
    read_text_preview_at(path, max_chars.unwrap_or(600))
}

fn read_text_file_at(path: String) -> Result<String, String> {
    read_text_file_at_from_dir(path, &crate::live::recordings::recordings_dir())
}

fn read_text_file_at_from_dir(path: String, owned_dir: &std::path::Path) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);
    let path = owned_live_transcript_path_from_dir(&path, "read", owned_dir)?;
    reject_oversized_transcript(&path)?;
    std::fs::read_to_string(&path).map_err(|err| format!("Failed to read transcript: {err}"))
}

fn read_text_preview_at(path: String, max_chars: usize) -> Result<String, String> {
    read_text_preview_at_from_dir(path, max_chars, &crate::live::recordings::recordings_dir())
}

fn read_text_preview_at_from_dir(
    path: String,
    max_chars: usize,
    owned_dir: &std::path::Path,
) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);

    let max_chars = max_chars.clamp(1, 4_000);
    let path = owned_live_transcript_path_from_dir(&path, "read", owned_dir)?;
    let file =
        std::fs::File::open(&path).map_err(|err| format!("Failed to read transcript: {err}"))?;
    let mut bytes = Vec::new();
    std::io::Read::take(file, (max_chars.saturating_mul(4).saturating_add(4)) as u64)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("Failed to read transcript: {err}"))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.chars().take(max_chars).collect())
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
    write_polished_text_at_from_dir(path, text, &crate::live::recordings::recordings_dir())
}

fn write_polished_text_at_from_dir(
    path: String,
    text: String,
    owned_dir: &std::path::Path,
) -> Result<String, String> {
    let path = std::path::PathBuf::from(path);
    let path = owned_live_transcript_path_from_dir(&path, "polished", owned_dir)?;
    let output = polished_path(&path)?;
    write_text_atomically(&output, &text)
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

fn write_text_atomically(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing file name")
        })?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    std::fs::remove_file(path.with_file_name(format!("{file_name}.part"))).ok();
    for attempt in 0..32 {
        let tmp = path.with_file_name(format!("{file_name}.{pid}.{nonce}.{attempt}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(mut file) => {
                let write_result = file.write_all(text.as_bytes());
                drop(file);
                let result = write_result.and_then(|_| std::fs::rename(&tmp, path));
                if result.is_err() {
                    std::fs::remove_file(&tmp).ok();
                }
                return result;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve temporary transcript path",
    ))
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
) -> Result<(), String> {
    ensure_main_window(&window)?;
    delete_history_entry_files_at(output_path)
}

fn delete_history_entry_files_at(output_path: String) -> Result<(), String> {
    delete_history_entry_files_at_from_dir(output_path, &crate::live::recordings::recordings_dir())
}

fn delete_history_entry_files_at_from_dir(
    output_path: String,
    owned_dir: &std::path::Path,
) -> Result<(), String> {
    let output = deletable_yap_owned_live_transcript_path_from_dir(output_path, owned_dir)?;
    let source = matching_owned_live_recording_path(&output);

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
    if !path.is_file() || !is_yap_media_or_transcript_path(&path) {
        return Err("Only Yap recording and transcript files can be opened.".into());
    }
    Ok(path)
}

fn deletable_yap_owned_live_transcript_path_from_dir(
    path: String,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(path);
    if !is_transcript_path(&path) {
        return Err("Only transcript text files can be deleted.".into());
    }
    let path = canonical_existing_path(&path)?;
    let owned_dir = owned_dir
        .canonicalize()
        .map_err(|_| "Only Yap-owned live transcripts can be deleted from device.".to_string())?;

    if !path.is_file() || !path.starts_with(&owned_dir) || !is_live_transcript_file(&path) {
        return Err("Only Yap-owned live transcripts can be deleted from device.".into());
    }
    Ok(path)
}

fn matching_owned_live_recording_path(output: &std::path::Path) -> Option<std::path::PathBuf> {
    let audio = output.with_extension("wav");
    audio.is_file().then_some(audio)
}

fn canonical_existing_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !path.exists() {
        return Err("File no longer exists.".into());
    }
    path.canonicalize()
        .map_err(|err| format!("Failed to resolve file path: {err}"))
}

fn canonical_transcript_path(
    path: &std::path::Path,
    action: &str,
) -> Result<std::path::PathBuf, String> {
    if !is_transcript_path(path) {
        return Err(format!("Only transcript text files can be {action}."));
    }
    let path = canonical_existing_path(path)?;
    if !path.is_file() || !is_transcript_path(&path) {
        return Err(format!("Only transcript text files can be {action}."));
    }
    Ok(path)
}

fn owned_live_transcript_path_from_dir(
    path: &std::path::Path,
    action: &str,
    owned_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let path = canonical_transcript_path(path, action)?;
    let owned_dir = owned_dir
        .canonicalize()
        .map_err(|_| format!("Only Yap-owned live transcripts can be {action}."))?;

    if !path.starts_with(&owned_dir) || !is_live_transcript_file(&path) {
        return Err(format!("Only Yap-owned live transcripts can be {action}."));
    }
    Ok(path)
}

fn reject_oversized_transcript(path: &std::path::Path) -> Result<(), String> {
    let length = std::fs::metadata(path)
        .map_err(|err| format!("Failed to inspect transcript: {err}"))?
        .len();
    if length > MAX_TRANSCRIPT_READ_BYTES {
        return Err(
            "Transcript is too large to load in the app. Open it from disk instead.".into(),
        );
    }
    Ok(())
}

pub(crate) fn ensure_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == crate::MAIN_WINDOW_LABEL {
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

fn is_live_transcript_file(path: &std::path::Path) -> bool {
    is_transcript_path(path)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.starts_with("live-"))
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
            crate::live::recordings::unix_millis_now().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_text_file_rejects_non_transcripts() {
        assert!(read_text_file_at("recording.mp3".into()).is_err());
    }

    #[test]
    fn read_text_preview_caps_transcript_text() {
        let dir = temp_test_dir("preview-cap");
        let transcript = dir.join("live-100.txt");
        std::fs::write(&transcript, "abcdef").unwrap();

        let preview =
            read_text_preview_at_from_dir(transcript.display().to_string(), 3, &dir).unwrap();

        assert_eq!(preview, "abc");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_text_preview_handles_multibyte_boundary() {
        let dir = temp_test_dir("preview-multibyte");
        let transcript = dir.join("live-105.txt");
        std::fs::write(&transcript, "abcdefg€").unwrap();

        let preview =
            read_text_preview_at_from_dir(transcript.display().to_string(), 1, &dir).unwrap();

        assert_eq!(preview, "a");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_read_rejects_directory_after_canonicalization() {
        let dir = temp_test_dir("txt-dir");
        let transcript_dir = dir.join("live-101.txt");
        std::fs::create_dir_all(&transcript_dir).unwrap();

        let error =
            read_text_file_at_from_dir(transcript_dir.display().to_string(), &dir).unwrap_err();

        assert_eq!(error, "Only transcript text files can be read.");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn read_text_file_rejects_oversized_transcripts() {
        let dir = temp_test_dir("oversized-read");
        let transcript = dir.join("live-102.txt");
        std::fs::write(
            &transcript,
            vec![b'a'; (MAX_TRANSCRIPT_READ_BYTES as usize) + 1],
        )
        .unwrap();

        let error = read_text_file_at_from_dir(transcript.display().to_string(), &dir).unwrap_err();

        assert_eq!(
            error,
            "Transcript is too large to load in the app. Open it from disk instead."
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_reads_reject_external_text_files() {
        let owned_dir = temp_test_dir("owned-live-read");
        let external_dir = temp_test_dir("external-transcript-read");
        let transcript = external_dir.join("live-103.txt");
        std::fs::write(&transcript, "secret").unwrap();

        assert_eq!(
            read_text_file_at_from_dir(transcript.display().to_string(), &owned_dir).unwrap_err(),
            "Only Yap-owned live transcripts can be read."
        );
        assert_eq!(
            read_text_preview_at_from_dir(transcript.display().to_string(), 10, &owned_dir)
                .unwrap_err(),
            "Only Yap-owned live transcripts can be read."
        );
        assert_eq!(
            write_polished_text_at_from_dir(
                transcript.display().to_string(),
                "safe".into(),
                &owned_dir,
            )
            .unwrap_err(),
            "Only Yap-owned live transcripts can be polished."
        );
        std::fs::remove_dir_all(owned_dir).ok();
        std::fs::remove_dir_all(external_dir).ok();
    }

    #[test]
    fn transcript_actions_reject_resolved_non_transcript_files() {
        let dir = temp_test_dir("txt-symlink");
        let target = dir.join("secret.json");
        let link = dir.join("live-104.txt");
        std::fs::write(&target, "{}").unwrap();
        if create_file_symlink(&target, &link).is_err() {
            std::fs::remove_dir_all(dir).ok();
            return;
        }

        assert_eq!(
            read_text_file_at_from_dir(link.display().to_string(), &dir).unwrap_err(),
            "Only transcript text files can be read."
        );
        assert_eq!(
            read_text_preview_at_from_dir(link.display().to_string(), 10, &dir).unwrap_err(),
            "Only transcript text files can be read."
        );
        assert_eq!(
            write_polished_text_at_from_dir(link.display().to_string(), "safe".into(), &dir)
                .unwrap_err(),
            "Only transcript text files can be polished."
        );
        std::fs::remove_dir_all(dir).ok();
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
    fn app_open_path_rejects_media_named_directories() {
        let dir = temp_test_dir("open-media-dir");
        let media_dir = dir.join("clip.wav");
        std::fs::create_dir_all(&media_dir).unwrap();

        let err = openable_app_path(media_dir.display().to_string()).unwrap_err();

        assert_eq!(
            err,
            "Only Yap recording and transcript files can be opened."
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_history_entry_files_removes_owned_live_audio() {
        let dir = temp_test_dir("delete-owned-live");
        let transcript = dir.join("live-300.txt");
        let audio = dir.join("live-300.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();

        delete_history_entry_files_at_from_dir(transcript.display().to_string(), &dir).unwrap();

        assert!(!transcript.exists());
        assert!(!audio.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_history_entry_files_keeps_imported_source_audio() {
        let owned_dir = temp_test_dir("delete-owned-dir");
        let imported_dir = temp_test_dir("delete-imported-source");
        let transcript = owned_dir.join("live-301.txt");
        let audio = imported_dir.join("clip.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();

        delete_history_entry_files_at_from_dir(transcript.display().to_string(), &owned_dir)
            .unwrap();

        assert!(!transcript.exists());
        assert!(audio.exists());
        std::fs::remove_dir_all(owned_dir).ok();
        std::fs::remove_dir_all(imported_dir).ok();
    }

    #[test]
    fn delete_history_entry_files_ignores_mismatched_owned_source() {
        let dir = temp_test_dir("delete-mismatched-owned-source");
        let transcript = dir.join("live-302.txt");
        let matching_audio = dir.join("live-302.wav");
        let other_audio = dir.join("live-303.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&matching_audio, b"RIFF").unwrap();
        std::fs::write(&other_audio, b"RIFF").unwrap();

        delete_history_entry_files_at_from_dir(transcript.display().to_string(), &dir).unwrap();

        assert!(!transcript.exists());
        assert!(!matching_audio.exists());
        assert!(other_audio.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_history_entry_files_ignores_directory_shaped_audio() {
        let dir = temp_test_dir("delete-audio-dir");
        let transcript = dir.join("live-304.txt");
        let audio_dir = dir.join("live-304.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::create_dir_all(&audio_dir).unwrap();

        delete_history_entry_files_at_from_dir(transcript.display().to_string(), &dir).unwrap();

        assert!(!transcript.exists());
        assert!(audio_dir.is_dir());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_history_entry_files_rejects_directory_shaped_transcript() {
        let dir = temp_test_dir("delete-transcript-dir");
        let transcript_dir = dir.join("live-305.txt");
        std::fs::create_dir_all(&transcript_dir).unwrap();

        let err =
            delete_history_entry_files_at_from_dir(transcript_dir.display().to_string(), &dir)
                .unwrap_err();

        assert!(err.contains("Yap-owned live transcripts"));
        assert!(transcript_dir.is_dir());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_history_entry_files_rejects_imported_transcript() {
        let owned_dir = temp_test_dir("delete-owned-dir");
        let imported_dir = temp_test_dir("delete-imported-transcript");
        let transcript = imported_dir.join("clip.txt");
        let audio = imported_dir.join("clip.wav");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();

        let err =
            delete_history_entry_files_at_from_dir(transcript.display().to_string(), &owned_dir)
                .unwrap_err();

        assert!(err.contains("Yap-owned live transcripts"));
        assert!(transcript.exists());
        assert!(audio.exists());
        std::fs::remove_dir_all(owned_dir).ok();
        std::fs::remove_dir_all(imported_dir).ok();
    }

    #[test]
    fn delete_history_entry_files_rejects_non_live_owned_transcript() {
        let dir = temp_test_dir("delete-non-live-owned");
        let transcript = dir.join("notes.txt");
        std::fs::write(&transcript, "do not delete\n").unwrap();

        let err = delete_history_entry_files_at_from_dir(transcript.display().to_string(), &dir)
            .unwrap_err();

        assert!(err.contains("Yap-owned live transcripts"));
        assert!(transcript.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn polished_path_writes_sibling_file() {
        let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
        assert_eq!(path.file_name().unwrap(), "take.polished.txt");
    }

    #[test]
    fn atomic_text_write_replaces_stale_partial_file() {
        let dir = temp_test_dir("atomic-polish-write");
        let output = dir.join("take.polished.txt");
        let partial = dir.join("take.polished.txt.part");
        std::fs::write(&partial, "stale").unwrap();

        write_text_atomically(&output, "polished").unwrap();

        assert_eq!(std::fs::read_to_string(&output).unwrap(), "polished");
        assert!(!partial.exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_text_write_replaces_existing_output() {
        let dir = temp_test_dir("atomic-polish-overwrite");
        let output = dir.join("take.polished.txt");
        std::fs::write(&output, "old").unwrap();

        write_text_atomically(&output, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&output).unwrap(), "new");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn atomic_text_write_uses_unique_temps_for_concurrent_writes() {
        let dir = temp_test_dir("atomic-polish-concurrent");
        let output = dir.join("take.polished.txt");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let left_output = output.clone();
        let left_barrier = std::sync::Arc::clone(&barrier);
        let left = std::thread::spawn(move || {
            left_barrier.wait();
            write_text_atomically(&left_output, "left")
        });
        let right_output = output.clone();
        let right_barrier = std::sync::Arc::clone(&barrier);
        let right = std::thread::spawn(move || {
            right_barrier.wait();
            write_text_atomically(&right_output, "right")
        });

        left.join().unwrap().unwrap();
        right.join().unwrap().unwrap();

        let text = std::fs::read_to_string(&output).unwrap();
        assert!(text == "left" || text == "right");
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "part")
            })
            .count();
        assert_eq!(leftovers, 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    fn create_file_symlink(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(
        target: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }
}
