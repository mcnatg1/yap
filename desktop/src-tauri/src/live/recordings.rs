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
    if transcript.is_none() && pcm.is_empty() {
        return Ok(None);
    }

    let dir = recordings_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create live recordings folder: {err}"))?;
    let created_at_ms = unix_millis_now()?;
    let name = format!("live-{created_at_ms}");
    let transcript_path = dir.join(format!("{name}.txt"));
    let audio_path = dir.join(format!("{name}.wav"));
    let transcript_body =
        transcript.unwrap_or_else(|| "Transcript unavailable for this live recording.".into());

    if !pcm.is_empty() {
        write_pcm16_wav(&audio_path, &pcm)?;
    }
    std::fs::write(&transcript_path, format!("{transcript_body}\n"))
        .map_err(|err| format!("Failed to save live transcript: {err}"))?;

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

        normalize_transcript(&path)?;

        let audio_path = path.with_extension("wav");
        let source_path = if audio_path.exists() {
            audio_path
        } else {
            path.clone()
        };
        let created_at_ms = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(system_time_to_unix_millis)
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

pub(crate) fn unix_millis_now() -> Result<u64, String> {
    system_time_to_unix_millis(std::time::SystemTime::now())
        .ok_or_else(|| "System clock error: timestamp out of range.".to_string())
}

fn system_time_to_unix_millis(time: std::time::SystemTime) -> Option<u64> {
    let millis = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    u64::try_from(millis).ok()
}

fn transcript_text(view: &live::state::LiveSessionView) -> Option<String> {
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

fn normalize_transcript(path: &std::path::Path) -> Result<(), String> {
    let current = std::fs::read_to_string(path)
        .map_err(|err| format!("Failed to read saved live transcript: {err}"))?;
    let cleaned = clean_transcript_text(&current);
    if cleaned.is_empty() || cleaned.trim_end() == current.trim_end() {
        return Ok(());
    }

    std::fs::write(path, format!("{cleaned}\n"))
        .map_err(|err| format!("Failed to repair saved live transcript: {err}"))
}

fn write_pcm16_wav(path: &std::path::Path, pcm: &[u8]) -> Result<(), String> {
    let data_len =
        u32::try_from(pcm.len()).map_err(|_| "Live recording is too large to save.".to_string())?;
    let riff_len = 36u32
        .checked_add(data_len)
        .ok_or_else(|| "Live recording is too large to save.".to_string())?;
    let byte_rate = LIVE_WAV_SAMPLE_RATE * 2;
    let mut file =
        std::fs::File::create(path).map_err(|err| format!("Failed to save live audio: {err}"))?;

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
    fn saved_live_session_scan_repairs_streaming_artifacts() {
        let dir = std::env::temp_dir().join(format!("yap-live-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-201.txt");
        std::fs::write(&transcript, "  THank   you.. \n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&transcript).unwrap(),
            "Thank you.\n"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_repairs_old_empty_placeholder() {
        let dir = std::env::temp_dir().join(format!("yap-live-placeholder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-202.txt");
        std::fs::write(&transcript, "No live transcript captured.\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&transcript).unwrap(),
            "Transcript unavailable for this live recording.\n"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
