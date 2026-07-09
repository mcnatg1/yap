use std::io::Write;

use crate::{file_actions, live};

const LIVE_WAV_SAMPLE_RATE: u32 = 16_000;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLiveSession {
    pub name: String,
    pub source_path: String,
    pub output_path: String,
    pub created_at_ms: u64,
}

pub fn save_session_files(
    live_runtime: &live::runtime::LiveRuntime,
    view: &live::state::LiveSessionView,
) -> Result<Option<SavedLiveSession>, String> {
    let transcript = transcript_text(view);
    let pcm = live_runtime.recorded_pcm();
    let created_at_ms = unix_millis_now()?;
    save_session_parts_to_dir(&recordings_dir(), created_at_ms, transcript, &pcm)
}

fn save_session_parts_to_dir(
    dir: &std::path::Path,
    created_at_ms: u64,
    transcript: Option<String>,
    pcm: &[u8],
) -> Result<Option<SavedLiveSession>, String> {
    if transcript.is_none() && pcm.is_empty() {
        return Ok(None);
    }

    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create live recordings folder: {err}"))?;
    let (name, transcript_path, audio_path) = reserve_session_paths(dir, created_at_ms)?;
    let transcript_body =
        transcript.unwrap_or_else(|| "Transcript unavailable for this live recording.".into());

    write_new_text_file(&transcript_path, &format!("{transcript_body}\n"))
        .map_err(|err| format!("Failed to save live transcript: {err}"))?;
    if !pcm.is_empty() {
        if write_pcm16_wav(&audio_path, pcm).is_err() {
            return Ok(Some(SavedLiveSession {
                name,
                source_path: transcript_path.display().to_string(),
                output_path: transcript_path.display().to_string(),
                created_at_ms,
            }));
        }
    }

    Ok(Some(SavedLiveSession {
        name,
        source_path: if pcm.is_empty() {
            transcript_path.display().to_string()
        } else {
            audio_path.display().to_string()
        },
        output_path: transcript_path.display().to_string(),
        created_at_ms,
    }))
}

fn reserve_session_paths(
    dir: &std::path::Path,
    created_at_ms: u64,
) -> Result<(String, std::path::PathBuf, std::path::PathBuf), String> {
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            format!("live-{created_at_ms}")
        } else {
            format!("live-{created_at_ms}-{suffix}")
        };
        let transcript_path = dir.join(format!("{name}.txt"));
        let audio_path = dir.join(format!("{name}.wav"));
        if !transcript_path.exists() && !audio_path.exists() {
            return Ok((name, transcript_path, audio_path));
        }
    }
    Err("Failed to allocate a live recording filename.".into())
}

pub fn list_session_files() -> Result<Vec<SavedLiveSession>, String> {
    list_session_files_from_dir(&recordings_dir())
}

fn recordings_dir_from<F>(env: F) -> std::path::PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = env("YAP_LIVE_RECORDINGS_DIR") {
        return std::path::PathBuf::from(dir);
    }
    if let Some(local) = env("LOCALAPPDATA") {
        return std::path::PathBuf::from(local)
            .join("Yap")
            .join("live-recordings");
    }
    std::path::PathBuf::from(".").join("live-recordings")
}

pub(crate) fn recordings_dir() -> std::path::PathBuf {
    recordings_dir_from(|key| std::env::var(key).ok())
}

fn list_session_files_from_dir(dir: &std::path::Path) -> Result<Vec<SavedLiveSession>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in
        std::fs::read_dir(dir).map_err(|err| format!("Failed to read live recordings: {err}"))?
    {
        let entry = entry.map_err(|err| format!("Failed to read live recording: {err}"))?;
        let path = entry.path();
        if !file_actions::is_transcript_path(&path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !stem.starts_with("live-") {
            continue;
        }

        let audio_path = path.with_extension("wav");
        let source_path = if audio_path.exists() {
            audio_path
        } else {
            path.clone()
        };
        let created_at_ms = entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(created_at_ms_from_live_stem)
            .or_else(|| {
                entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(system_time_to_unix_millis)
            })
            .unwrap_or(0);
        sessions.push(SavedLiveSession {
            name: stem.to_string(),
            source_path: source_path.display().to_string(),
            output_path: path.display().to_string(),
            created_at_ms,
        });
    }

    sessions.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(sessions)
}

fn created_at_ms_from_live_stem(stem: &str) -> Option<u64> {
    stem.strip_prefix("live-")?.split('-').next()?.parse().ok()
}

pub(crate) fn unix_millis_now() -> Result<u64, String> {
    system_time_to_unix_millis(std::time::SystemTime::now())
        .ok_or_else(|| "System clock error: timestamp out of range.".to_string())
}

fn system_time_to_unix_millis(time: std::time::SystemTime) -> Option<u64> {
    let millis = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    u64::try_from(millis).ok()
}

pub(crate) fn transcript_text(view: &live::state::LiveSessionView) -> Option<String> {
    view.final_text
        .as_deref()
        .or(view.partial_text.as_deref())
        .map(clean_transcript_text)
        .filter(|text| !text.is_empty())
}

fn clean_transcript_text(text: &str) -> String {
    if text.trim() == "No live transcript captured." {
        return "Transcript unavailable for this live recording.".into();
    }

    let mut cleaned = text
        .split_whitespace()
        .map(fix_word_casing)
        .collect::<Vec<_>>()
        .join(" ");
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    cleaned
}

fn fix_word_casing(word: &str) -> String {
    let mut chars = word.chars();
    let (Some(first), Some(second), Some(third)) = (chars.next(), chars.next(), chars.next())
    else {
        return word.to_string();
    };

    if first.is_uppercase() && second.is_uppercase() && third.is_lowercase() {
        let mut fixed = String::new();
        fixed.push(first);
        fixed.extend(second.to_lowercase());
        fixed.push(third);
        fixed.extend(chars);
        fixed
    } else {
        word.to_string()
    }
}

fn write_new_text_file(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(text.as_bytes())
}

fn write_pcm16_wav(path: &std::path::Path, pcm: &[u8]) -> Result<(), String> {
    let partial = partial_wav_path(path)?;
    std::fs::remove_file(&partial).ok();
    if let Err(err) = write_pcm16_wav_bytes(&partial, pcm) {
        std::fs::remove_file(&partial).ok();
        return Err(err);
    }
    std::fs::rename(&partial, path).map_err(|err| {
        std::fs::remove_file(&partial).ok();
        format!("Failed to finalize live audio: {err}")
    })
}

fn partial_wav_path(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Live recording path has no file name.".to_string())?;
    Ok(path.with_file_name(format!("{file_name}.part")))
}

fn write_pcm16_wav_bytes(path: &std::path::Path, pcm: &[u8]) -> Result<(), String> {
    let data_len =
        u32::try_from(pcm.len()).map_err(|_| "Live recording is too large to save.".to_string())?;
    let riff_len = 36u32
        .checked_add(data_len)
        .ok_or_else(|| "Live recording is too large to save.".to_string())?;
    let byte_rate = LIVE_WAV_SAMPLE_RATE * 2;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("Failed to save live audio: {err}"))?;

    file.write_all(b"RIFF").map_err(wav_write_error)?;
    file.write_all(&riff_len.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(b"WAVEfmt ").map_err(wav_write_error)?;
    file.write_all(&16u32.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&1u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&1u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&LIVE_WAV_SAMPLE_RATE.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&byte_rate.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&2u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(&16u16.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(b"data").map_err(wav_write_error)?;
    file.write_all(&data_len.to_le_bytes())
        .map_err(wav_write_error)?;
    file.write_all(pcm).map_err(wav_write_error)
}

fn wav_write_error(err: std::io::Error) -> String {
    format!("Failed to save live audio: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_view(
        final_text: Option<&str>,
        partial_text: Option<&str>,
    ) -> live::state::LiveSessionView {
        live::state::LiveSessionView {
            visibility: live::state::LiveOverlayVisibility::Enabled,
            status: live::state::LiveSessionStatus::Idle,
            route: live::state::LiveRoute::None,
            capture_mode: live::state::LiveCaptureMode::PushToTalk,
            active_capture_mode: None,
            hotkey: String::new(),
            paste_hotkey: String::new(),
            input_device_id: None,
            input_device_label: None,
            level: None,
            partial_text: partial_text.map(str::to_string),
            final_text: final_text.map(str::to_string),
            error: None,
        }
    }

    #[test]
    fn transcript_text_prefers_final_then_partial() {
        let mut view = live_view(Some("final"), Some("partial"));

        assert_eq!(transcript_text(&view).as_deref(), Some("final"));
        view.final_text = None;
        assert_eq!(transcript_text(&view).as_deref(), Some("partial"));
    }

    #[test]
    fn transcript_text_cleans_streaming_artifacts() {
        let mut view = live_view(Some("  THank   you.. "), None);

        assert_eq!(transcript_text(&view).as_deref(), Some("Thank you."));
        view.final_text = Some("NASA called.".into());
        assert_eq!(transcript_text(&view).as_deref(), Some("NASA called."));
    }

    #[test]
    fn write_pcm16_wav_writes_standard_header_and_data() {
        let path = std::env::temp_dir().join(format!("yap-live-{}.wav", std::process::id()));
        let pcm = [0, 0, 255, 127];
        std::fs::remove_file(&path).ok();

        write_pcm16_wav(&path, &pcm).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 4);
        assert_eq!(&bytes[44..], pcm);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn write_pcm16_wav_replaces_stale_partial_file() {
        let path = std::env::temp_dir().join(format!("yap-live-stale-{}.wav", std::process::id()));
        let partial = partial_wav_path(&path).unwrap();
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&partial).ok();
        std::fs::write(&partial, b"stale").unwrap();

        write_pcm16_wav(&path, &[1, 0]).unwrap();

        assert!(path.exists());
        assert!(!partial.exists());
        assert_ne!(std::fs::read(&path).unwrap(), b"stale");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn write_pcm16_wav_does_not_create_final_file_when_partial_fails() {
        let dir = std::env::temp_dir().join(format!("yap-live-partial-dir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.wav");
        let partial = partial_wav_path(&path).unwrap();
        std::fs::create_dir_all(&partial).unwrap();

        let err = write_pcm16_wav(&path, &[1, 0]).unwrap_err();

        assert!(err.contains("Failed to save live audio"));
        assert!(!path.exists());
        assert!(partial.is_dir());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_pairs_transcripts_with_audio() {
        let dir = std::env::temp_dir().join(format!("yap-live-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-200.txt");
        let audio = dir.join("live-200.wav");
        let ignored = dir.join("note.txt");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();
        std::fs::write(&ignored, "not a live session\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "live-200");
        assert_eq!(sessions[0].output_path, transcript.display().to_string());
        assert_eq!(sessions[0].source_path, audio.display().to_string());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_does_not_rewrite_streaming_artifacts() {
        let dir = std::env::temp_dir().join(format!("yap-live-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-201.txt");
        std::fs::write(&transcript, "  THank   you.. \n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&transcript).unwrap(),
            "  THank   you.. \n"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_does_not_rewrite_old_empty_placeholder() {
        let dir = std::env::temp_dir().join(format!("yap-live-placeholder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-202.txt");
        std::fs::write(&transcript, "No live transcript captured.\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&transcript).unwrap(),
            "No live transcript captured.\n"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_session_parts_avoids_same_millisecond_overwrite() {
        let dir = std::env::temp_dir().join(format!("yap-live-collision-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let first = save_session_parts_to_dir(&dir, 123, Some("first".into()), &[])
            .unwrap()
            .unwrap();
        let second = save_session_parts_to_dir(&dir, 123, Some("second".into()), &[])
            .unwrap()
            .unwrap();

        assert_ne!(first.name, second.name);
        assert_eq!(
            std::fs::read_to_string(first.output_path).unwrap(),
            "first\n"
        );
        assert_eq!(
            std::fs::read_to_string(second.output_path).unwrap(),
            "second\n"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_uses_filename_timestamp() {
        let dir = std::env::temp_dir().join(format!("yap-live-timestamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-999-1.txt");
        std::fs::write(&transcript, "hello\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].created_at_ms, 999);
        std::fs::remove_dir_all(dir).ok();
    }
}
