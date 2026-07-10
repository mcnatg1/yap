use std::io::Write;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::evidence::ModelRevision;
use crate::audio::recording::{self, RecordingFinalizeResult};
use crate::audio::results::{ResultAuthority, ResultStatus, TranscriptResultRevision};
use crate::{file_actions, live};

const AUDIO_SAVE_FAILED_WARNING: &str = "Live audio could not be saved. Transcript was saved.";
const TRANSCRIPT_DEGRADED_WARNING: &str = "Live transcript may be incomplete. Audio was saved.";

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLiveSession {
    pub name: String,
    pub source_path: String,
    pub output_path: String,
    pub created_at_ms: u64,
    pub warning: Option<String>,
}

pub fn save_session_files(
    live_runtime: &live::runtime::LiveRuntime,
    view: &live::state::LiveSessionView,
) -> Result<Option<SavedLiveSession>, String> {
    save_session_files_to_dir(live_runtime, view, &recordings_dir())
}

fn save_session_files_to_dir(
    live_runtime: &live::runtime::LiveRuntime,
    view: &live::state::LiveSessionView,
    dir: &std::path::Path,
) -> Result<Option<SavedLiveSession>, String> {
    let capture = live_runtime.finalize_recording()?;
    save_finalized_capture_to_dir(dir, view, capture)
}

fn save_finalized_capture_to_dir(
    dir: &std::path::Path,
    view: &live::state::LiveSessionView,
    capture: Option<RecordingFinalizeResult>,
) -> Result<Option<SavedLiveSession>, String> {
    let Some(capture) = capture else {
        return Ok(None);
    };
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("Failed to create live recordings folder: {err}"))?;
    let name = format!("live-{}", capture.session_id);
    let transcript_path = dir.join(format!("{name}.txt"));
    let transcript = transcript_text(view)
        .unwrap_or_else(|| "Transcript unavailable for this live recording.".into());
    write_new_text_file(&transcript_path, &format!("{transcript}\n"))
        .map_err(|error| format!("Failed to save live transcript: {error}"))?;

    let warning = view
        .transcription_degraded
        .then_some(TRANSCRIPT_DEGRADED_WARNING.to_string());
    let created_at_ms = unix_millis_now()?;
    let output_path = stable_existing_path_string(&transcript_path);
    let status =
        if capture.status == recording::CaptureStatus::Partial || view.transcription_degraded {
            ResultStatus::Partial
        } else {
            ResultStatus::Complete
        };
    let revision_warning = capture
        .capture_sidecar_sha256()
        .ok_or_else(|| "Capture lineage is unavailable for the transcript revision".to_string())
        .and_then(|capture_sidecar_sha256| {
            write_transcript_revision(
                dir,
                &capture.session_id,
                capture_sidecar_sha256,
                &transcript_path,
                &transcript,
                status,
            )
        })
        .err();
    let Some(committed) = capture.committed else {
        return Ok(Some(SavedLiveSession {
            name,
            source_path: output_path.clone(),
            output_path,
            created_at_ms,
            warning: revision_warning.map_or_else(
                || combine_warning(warning.clone(), AUDIO_SAVE_FAILED_WARNING),
                |error| {
                    combine_warning(
                        combine_warning(warning.clone(), AUDIO_SAVE_FAILED_WARNING),
                        format!("Transcript revision was not saved: {error}"),
                    )
                },
            ),
        }));
    };
    let audio_path = dir.join(&committed.manifest.audio_file);
    Ok(Some(SavedLiveSession {
        name,
        source_path: stable_existing_path_string(&audio_path),
        output_path,
        created_at_ms,
        warning: revision_warning.map_or(warning.clone(), |error| {
            combine_warning(
                warning,
                format!("Transcript revision was not saved: {error}"),
            )
        }),
    }))
}

fn combine_warning(base: Option<String>, next: impl AsRef<str>) -> Option<String> {
    let next = next.as_ref();
    if next.is_empty() {
        return base;
    }
    match base {
        Some(base) if !base.is_empty() => Some(format!("{base} {next}")),
        _ => Some(next.to_string()),
    }
}

pub fn list_session_files() -> Result<Vec<SavedLiveSession>, String> {
    list_session_files_from_dir(&recordings_dir())
}

fn recordings_dir_from<F>(env: F) -> std::path::PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = crate::paths::absolute_env_path(&env, "YAP_LIVE_RECORDINGS_DIR") {
        return dir;
    }
    crate::paths::app_data_dir_from(env).join("live-recordings")
}

pub(crate) fn recordings_dir() -> std::path::PathBuf {
    recordings_dir_from(|key| std::env::var(key).ok())
}

fn list_session_files_from_dir(dir: &std::path::Path) -> Result<Vec<SavedLiveSession>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = recording::scan_recordings(dir)?
        .complete
        .into_iter()
        .map(|committed| {
            let name = format!("live-{}", committed.manifest.session_id);
            let transcript = dir.join(format!("{name}.txt"));
            let audio = dir.join(&committed.manifest.audio_file);
            SavedLiveSession {
                name,
                source_path: stable_existing_path_string(&audio),
                output_path: if transcript.is_file() {
                    stable_existing_path_string(&transcript)
                } else {
                    stable_existing_path_string(&audio)
                },
                created_at_ms: committed_at_ms(&committed.manifest.committed_at_utc),
                warning: None,
            }
        })
        .collect::<Vec<_>>();
    for entry in
        std::fs::read_dir(dir).map_err(|err| format!("Failed to read live recordings: {err}"))?
    {
        let entry = entry.map_err(|err| format!("Failed to read live recording: {err}"))?;
        let path = entry.path();
        if !path.is_file() || !is_primary_live_transcript_path(&path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if dir.join(format!("{stem}.capture.journal.part")).exists()
            || dir.join(format!("{stem}.capture.json")).exists()
            || dir.join(format!("{stem}.capture.partial.json")).exists()
            || dir.join(format!("{stem}.commit.json")).exists()
        {
            continue;
        }
        let created_at_ms = created_at_ms_from_live_stem(stem)
            .expect("primary live transcript predicate validates the stem");

        let audio_path = path.with_extension("wav");
        let source_path = if audio_path.is_file() {
            audio_path
        } else {
            path.clone()
        };
        sessions.push(SavedLiveSession {
            name: stem.to_string(),
            source_path: stable_existing_path_string(&source_path),
            output_path: stable_existing_path_string(&path),
            created_at_ms,
            warning: None,
        });
    }

    sessions.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(sessions)
}

fn committed_at_ms(value: &str) -> u64 {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .and_then(|timestamp| u64::try_from(timestamp.unix_timestamp_nanos() / 1_000_000).ok())
        .unwrap_or(0)
}

fn write_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    capture_sidecar_sha256: &str,
    transcript_path: &std::path::Path,
    transcript: &str,
    status: ResultStatus,
) -> Result<(), String> {
    let revision = next_transcript_revision(dir, session_id)?;
    let model = ModelRevision::new(crate::stt::nemotron::MODEL_ID, "local", "local")
        .map_err(|error| format!("Failed to describe local transcript model: {error}"))?;
    let result = if revision == 1 {
        TranscriptResultRevision::new(
            session_id.clone(),
            revision,
            ResultAuthority::LocalProvisional,
            capture_sidecar_sha256,
            None,
            status,
            transcript,
            Vec::new(),
            vec![model],
        )
    } else {
        let previous_path = transcript_revision_path(dir, session_id, revision - 1);
        let previous_text = std::fs::read_to_string(&previous_path)
            .map_err(|error| format!("Failed to read prior transcript revision: {error}"))?;
        let previous: TranscriptResultRevision = serde_json::from_str(&previous_text)
            .map_err(|error| format!("Failed to parse prior transcript revision: {error}"))?;
        previous.next_revision(
            revision,
            ResultAuthority::LocalProvisional,
            capture_sidecar_sha256,
            recording::sha256_file(&previous_path)?,
            status,
            transcript,
            Vec::new(),
            vec![model],
        )
    }
    .map_err(|error| format!("Failed to build transcript revision: {error}"))?;
    let path = transcript_revision_path(dir, session_id, revision);
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("Failed to create transcript revision: {error}"))?;
    let serialized = transcript_result_value(&result, capture_sidecar_sha256, transcript_path)?;
    serde_json::to_writer(&mut file, &serialized)
        .map_err(|error| format!("Failed to write transcript revision: {error}"))?;
    file.write_all(b"\n")
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Failed to finalize transcript revision: {error}"))?;
    Ok(())
}

fn transcript_result_value(
    result: &TranscriptResultRevision,
    capture_sidecar_sha256: &str,
    transcript_path: &std::path::Path,
) -> Result<serde_json::Value, String> {
    let text_file = transcript_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Transcript path has no valid file name".to_string())?;
    recording::validate_artifact_name(text_file)?;
    let mut value = serde_json::to_value(result)
        .map_err(|error| format!("Failed to serialize transcript revision: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Transcript revision did not serialize as an object".to_string())?;
    object.insert("schemaVersion".into(), serde_json::Value::from(1u16));
    object.insert("textFile".into(), serde_json::Value::from(text_file));
    object.insert(
        "textSha256".into(),
        serde_json::Value::from(recording::sha256_file(transcript_path)?),
    );
    object.insert(
        "modelId".into(),
        serde_json::Value::from(crate::stt::nemotron::MODEL_ID),
    );
    object.insert("modelRevision".into(), serde_json::Value::from("local"));
    object.insert(
        "createdAtUtc".into(),
        serde_json::Value::from(
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|_| "Failed to format transcript revision time")?,
        ),
    );
    object.insert(
        "captureSidecarSha256".into(),
        serde_json::Value::from(capture_sidecar_sha256),
    );
    Ok(value)
}

fn next_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<u64, String> {
    let prefix = format!("live-{session_id}.transcript.r");
    let mut highest = 0;
    for entry in std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to read transcript revisions: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("Failed to read transcript revision: {error}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if let Some(revision) = name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".json"))
            .and_then(|value| value.parse::<u64>().ok())
        {
            highest = highest.max(revision);
        }
    }
    highest
        .checked_add(1)
        .ok_or_else(|| "Transcript revision overflowed".into())
}

fn transcript_revision_path(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    revision: u64,
) -> std::path::PathBuf {
    dir.join(format!("live-{session_id}.transcript.r{revision}.json"))
}

pub(crate) fn stable_existing_path_string(path: &std::path::Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    stable_path_string(&canonical)
}

#[cfg(target_os = "windows")]
fn stable_path_string(path: &std::path::Path) -> String {
    let display = path.display().to_string();
    if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    display
        .strip_prefix(r"\\?\")
        .unwrap_or(&display)
        .to_string()
}

#[cfg(not(target_os = "windows"))]
fn stable_path_string(path: &std::path::Path) -> String {
    path.display().to_string()
}

pub(crate) fn is_primary_live_transcript_path(path: &std::path::Path) -> bool {
    file_actions::is_transcript_path(path)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(created_at_ms_from_live_stem)
            .is_some()
}

fn created_at_ms_from_live_stem(stem: &str) -> Option<u64> {
    let mut parts = stem.strip_prefix("live-")?.split('-');
    let created_at_ms = parts.next()?.parse().ok()?;
    match parts.next() {
        None => Some(created_at_ms),
        Some(suffix) if suffix.parse::<u16>().is_ok() && parts.next().is_none() => {
            Some(created_at_ms)
        }
        _ => None,
    }
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

pub(crate) fn completed_transcript_text(view: &live::state::LiveSessionView) -> Option<String> {
    view.final_text
        .as_deref()
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
    write_new_text_file_with(
        path,
        text,
        |file| file.sync_all(),
        |from, to| std::fs::rename(from, to),
    )
}

fn write_new_text_file_with<S, R>(
    path: &std::path::Path,
    text: &str,
    sync: S,
    rename: R,
) -> std::io::Result<()>
where
    S: FnOnce(&std::fs::File) -> std::io::Result<()>,
    R: FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
{
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "live transcript already exists",
        ));
    }
    let partial = partial_text_path(path)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let result = file.write_all(text.as_bytes()).and_then(|_| sync(&file));
    drop(file);
    let result = result.and_then(|_| {
        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "live transcript already exists",
            ));
        }
        rename(&partial, path)
    });
    if result.is_err() {
        std::fs::remove_file(&partial).ok();
    }
    result
}

fn partial_text_path(path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "missing transcript file name",
            )
        })?;
    Ok(path.with_file_name(format!("{file_name}.part")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::recording::{CommitFaultPoint, StreamingRecording};
    use crate::audio::session::SessionId;

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
            transcription_degraded: false,
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
    fn completed_transcript_text_never_promotes_a_partial() {
        let mut view = live_view(None, Some("partial"));
        assert_eq!(completed_transcript_text(&view), None);

        view.final_text = Some("final".into());
        assert_eq!(completed_transcript_text(&view).as_deref(), Some("final"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn stable_path_strings_remove_windows_verbatim_prefixes() {
        assert_eq!(
            stable_path_string(std::path::Path::new(r"\\?\C:\Users\Me\live-1.txt")),
            r"C:\Users\Me\live-1.txt"
        );
        assert_eq!(
            stable_path_string(std::path::Path::new(r"\\?\UNC\server\share\live-1.txt")),
            r"\\server\share\live-1.txt"
        );
    }

    #[test]
    fn transcript_text_cleans_streaming_artifacts() {
        let mut view = live_view(Some("  THank   you.. "), None);

        assert_eq!(transcript_text(&view).as_deref(), Some("Thank you."));
        view.final_text = Some("NASA called.".into());
        assert_eq!(transcript_text(&view).as_deref(), Some("NASA called."));
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
    fn new_style_uncommitted_artifacts_are_not_listed_as_legacy_history() {
        let dir = test_dir("uncommitted-new-style");
        let session = SessionId::new("s-pending").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        std::fs::write(dir.join(format!("live-{session}.txt")), "pending\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn committed_capture_is_listed_only_after_manifest_validation() {
        let dir = test_dir("committed-history");
        let session = SessionId::new("s-history").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        save_finalized_capture_to_dir(&dir, &live_view(Some("hello"), None), Some(capture))
            .unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, format!("live-{session}"));
        assert!(sessions[0].source_path.ends_with(".wav"));
        assert!(sessions[0].created_at_ms > 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn partial_capture_before_sidecar_publication_keeps_transcript_and_publishes_partial_revision()
    {
        assert_partial_capture_transcript(CommitFaultPoint::AudioSync);
    }

    #[test]
    fn partial_capture_after_sidecar_publication_keeps_transcript_and_publishes_partial_revision() {
        assert_partial_capture_transcript(CommitFaultPoint::CommitSync);
    }

    #[test]
    fn transcript_sync_failure_does_not_rename_the_partial_file() {
        let dir = test_dir("transcript-sync-failure");
        let transcript = dir.join("live-301.txt");
        let renamed = std::cell::Cell::new(false);

        let error = write_new_text_file_with(
            &transcript,
            "hello\n",
            |_| Err(std::io::Error::other("injected transcript sync failure")),
            |from, to| {
                renamed.set(true);
                std::fs::rename(from, to)
            },
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert!(!renamed.get());
        assert!(!transcript.exists());
        assert!(!partial_text_path(&transcript).unwrap().exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_revisions_are_create_new_and_monotonic() {
        let dir = test_dir("transcript-revisions");
        let session = SessionId::new("s-revisions").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let manifest = recording.finalize().unwrap().committed.unwrap().manifest;
        let text_path = dir.join(format!("live-{session}.txt"));
        write_new_text_file(&text_path, "first\n").unwrap();

        write_transcript_revision(
            &dir,
            &manifest.session_id,
            &manifest.capture_sidecar_sha256,
            &text_path,
            "first",
            ResultStatus::Complete,
        )
        .unwrap();
        write_transcript_revision(
            &dir,
            &manifest.session_id,
            &manifest.capture_sidecar_sha256,
            &text_path,
            "second",
            ResultStatus::Complete,
        )
        .unwrap();

        assert!(transcript_revision_path(&dir, &session, 1).is_file());
        assert!(transcript_revision_path(&dir, &session, 2).is_file());
        let revision =
            std::fs::read_to_string(transcript_revision_path(&dir, &session, 1)).unwrap();
        let revision: serde_json::Value = serde_json::from_str(&revision).unwrap();
        assert_eq!(revision["textFile"], format!("live-{session}.txt"));
        assert_eq!(revision["textSha256"].as_str().unwrap().len(), 64);
        assert_eq!(revision["modelId"], crate::stt::nemotron::MODEL_ID);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_ignores_polished_and_malformed_names() {
        let dir =
            std::env::temp_dir().join(format!("yap-live-primary-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "live-205.txt",
            "live-205-1.txt",
            "live-205.polished.txt",
            "live-not-a-time.txt",
            "live-205-extra-part.txt",
        ] {
            std::fs::write(dir.join(name), "hello\n").unwrap();
        }

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(
            sessions
                .iter()
                .map(|session| session.name.as_str())
                .collect::<Vec<_>>(),
            vec!["live-205-1", "live-205"]
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_ignores_directory_shaped_entries() {
        let dir = std::env::temp_dir().join(format!("yap-live-dir-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript_dir = dir.join("live-203.txt");
        let transcript = dir.join("live-204.txt");
        let audio_dir = dir.join("live-204.wav");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::create_dir_all(&audio_dir).unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].output_path, transcript.display().to_string());
        assert_eq!(sessions[0].source_path, transcript.display().to_string());
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
    fn recordings_dir_uses_absolute_override_or_app_data() {
        let override_dir = std::env::temp_dir().join("custom-live-recordings");
        assert_eq!(
            recordings_dir_from(|key| (key == "YAP_LIVE_RECORDINGS_DIR")
                .then(|| override_dir.display().to_string())),
            override_dir
        );

        let local = std::env::temp_dir().join("local-data");
        assert_eq!(
            recordings_dir_from(|key| match key {
                "YAP_LIVE_RECORDINGS_DIR" => Some("relative-live-recordings".into()),
                "LOCALAPPDATA" => Some(local.display().to_string()),
                _ => None,
            }),
            local.join("Yap").join("live-recordings")
        );
    }

    #[test]
    fn write_new_text_file_does_not_scan_partial_transcripts() {
        let dir =
            std::env::temp_dir().join(format!("yap-live-text-partial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-77.txt");
        let partial = partial_text_path(&transcript).unwrap();
        std::fs::write(&partial, "stale").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
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

    fn test_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yap-live-{label}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn assert_partial_capture_transcript(fault: CommitFaultPoint) {
        let dir = test_dir(&format!("partial-transcript-{fault:?}"));
        let session = SessionId::new("s-partial-transcript").unwrap();
        let mut recording =
            StreamingRecording::create_with_fault(&dir, session.clone(), fault).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        let lineage_hash = capture.capture_sidecar_sha256().unwrap().to_string();
        let lineage_file = capture
            .partial_lineage
            .as_ref()
            .map(|lineage| lineage.capture_sidecar_file.clone())
            .or_else(|| {
                capture
                    .committed
                    .as_ref()
                    .map(|committed| committed.manifest.capture_sidecar_file.clone())
            })
            .unwrap();

        let saved = save_finalized_capture_to_dir(
            &dir,
            &live_view(Some("transcript survives"), None),
            Some(capture),
        )
        .unwrap()
        .unwrap();

        let transcript = dir.join(format!("live-{session}.txt"));
        let revision = transcript_revision_path(&dir, &session, 1);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&revision).unwrap()).unwrap();
        assert_eq!(
            std::fs::read_to_string(transcript).unwrap(),
            "transcript survives\n"
        );
        assert_eq!(value["status"], "partial");
        assert_eq!(value["captureSidecarSha256"], lineage_hash);
        assert_eq!(
            recording::sha256_file(&dir.join(lineage_file)).unwrap(),
            lineage_hash
        );
        assert!(saved.warning.unwrap().contains(AUDIO_SAVE_FAILED_WARNING));
        let scanned = recording::scan_recordings(&dir).unwrap();
        assert!(scanned.complete.is_empty());
        assert_eq!(scanned.partial.len(), 1);
        assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
