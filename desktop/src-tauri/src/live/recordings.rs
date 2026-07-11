use std::io::Write;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::evidence::ModelRevision;
use crate::audio::recording::{self, PublishedTranscriptReceipt, RecordingFinalizeResult};
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
    match live_runtime.finalize_recording() {
        Ok(capture) => save_finalized_capture_to_dir(dir, view, capture),
        Err(error) => {
            let (session_id, cached_error) =
                live_runtime.recording_finalization_failure().ok_or(error)?;
            save_unavailable_capture_transcript_to_dir(dir, view, session_id, cached_error)
        }
    }
}

fn save_unavailable_capture_transcript_to_dir(
    dir: &std::path::Path,
    view: &live::state::LiveSessionView,
    session_id: crate::audio::session::SessionId,
    capture_error: String,
) -> Result<Option<SavedLiveSession>, String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("Failed to create live recordings folder: {err}"))?;
    let name = format!("live-{session_id}");
    let transcript_path = dir.join(format!("{name}.txt"));
    let transcript = transcript_text(view)
        .unwrap_or_else(|| "Transcript unavailable for this live recording.".into());
    write_new_text_file(&transcript_path, &format!("{transcript}\n"))
        .map_err(|error| format!("Failed to save live transcript: {error}"))?;
    let output_path = stable_existing_path_string(&transcript_path);
    let warning = combine_warning(
        view.transcription_degraded
            .then_some(TRANSCRIPT_DEGRADED_WARNING.to_string()),
        AUDIO_SAVE_FAILED_WARNING,
    );
    Ok(Some(SavedLiveSession {
        name,
        source_path: output_path.clone(),
        output_path,
        created_at_ms: unix_millis_now()?,
        warning: combine_warning(
            warning,
            format!("Capture finalization failed: {capture_error}"),
        ),
    }))
}

fn save_finalized_capture_to_dir(
    dir: &std::path::Path,
    view: &live::state::LiveSessionView,
    capture: Option<RecordingFinalizeResult>,
) -> Result<Option<SavedLiveSession>, String> {
    save_finalized_capture_to_dir_with_text_publisher(
        dir,
        view,
        capture,
        |source, destination, owned| {
            recording::publish_no_replace(source, destination, owned, "publish live transcript")
        },
    )
}

fn save_finalized_capture_to_dir_with_text_publisher<P>(
    dir: &std::path::Path,
    view: &live::state::LiveSessionView,
    capture: Option<RecordingFinalizeResult>,
    publisher: P,
) -> Result<Option<SavedLiveSession>, String>
where
    P: FnOnce(&std::path::Path, &std::path::Path, &std::fs::File) -> Result<std::fs::File, String>,
{
    let Some(capture) = capture else {
        return Ok(None);
    };
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("Failed to create live recordings folder: {err}"))?;
    let name = format!("live-{}", capture.session_id);
    let transcript_path = dir.join(format!("{name}.txt"));
    let transcript = transcript_text(view)
        .unwrap_or_else(|| "Transcript unavailable for this live recording.".into());
    let transcript_receipt = write_new_text_file_with(
        &transcript_path,
        &format!("{transcript}\n"),
        |file| file.sync_all(),
        publisher,
    )
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
        .revalidate_capture_sidecar()
        .and_then(|_| {
            capture.capture_sidecar_sha256().ok_or_else(|| {
                "Capture lineage is unavailable for the transcript revision".to_string()
            })
        })
        .and_then(|capture_sidecar_sha256| {
            write_transcript_revision(
                dir,
                &capture.session_id,
                capture_sidecar_sha256,
                &transcript_receipt,
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
                output_path: if has_valid_transcript_revision(
                    dir,
                    &committed.manifest.session_id,
                    &committed.manifest.capture_sidecar_sha256,
                ) && recording::is_regular_artifact(&transcript)
                {
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
        let name = entry.file_name();
        let Some(file_name) = name.to_str() else {
            continue;
        };
        let path = dir.join(file_name);
        if !recording::is_regular_artifact(&path) || !is_primary_live_transcript_path(&path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if regular_artifact_exists(dir, &format!("{stem}.capture.journal.part"))
            || regular_artifact_exists(dir, &format!("{stem}.capture.json"))
            || regular_artifact_exists(dir, &format!("{stem}.capture.partial.json"))
            || regular_artifact_exists(dir, &format!("{stem}.commit.json"))
        {
            continue;
        }
        let created_at_ms = created_at_ms_from_live_stem(stem)
            .expect("primary live transcript predicate validates the stem");

        let audio_path = path.with_extension("wav");
        let source_path = if recording::is_regular_artifact(&audio_path) {
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

fn regular_artifact_exists(dir: &std::path::Path, name: &str) -> bool {
    recording::open_regular_artifact(dir, name).is_ok()
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
    transcript_receipt: &PublishedTranscriptReceipt,
    transcript: &str,
    status: ResultStatus,
) -> Result<(), String> {
    write_transcript_revision_with_barrier(
        dir,
        session_id,
        capture_sidecar_sha256,
        transcript_receipt,
        transcript,
        status,
        |_| {},
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptRevisionPublicationBarrier {
    BeforePublication,
    AfterPublication,
}

fn write_transcript_revision_with_barrier<F>(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    capture_sidecar_sha256: &str,
    transcript_receipt: &PublishedTranscriptReceipt,
    transcript: &str,
    status: ResultStatus,
    mut publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(TranscriptRevisionPublicationBarrier),
{
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
        let previous_name = format!("live-{session_id}.transcript.r{}.json", revision - 1);
        let (previous_text, previous_sha256) =
            recording::read_and_hash_regular_artifact(dir, &previous_name)
                .map_err(|error| format!("Failed to read prior transcript revision: {error}"))?;
        let previous: TranscriptResultRevision = serde_json::from_str(&previous_text)
            .map_err(|error| format!("Failed to parse prior transcript revision: {error}"))?;
        previous.next_revision(
            revision,
            ResultAuthority::LocalProvisional,
            capture_sidecar_sha256,
            previous_sha256,
            status,
            transcript,
            Vec::new(),
            vec![model],
        )
    }
    .map_err(|error| format!("Failed to build transcript revision: {error}"))?;
    let path = transcript_revision_path(dir, session_id, revision);
    let serialized = transcript_result_value(&result, capture_sidecar_sha256, transcript_receipt)?;
    let (staging, mut file) = create_unique_transcript_revision_staging(dir, session_id, revision)?;
    let result = (|| {
        serde_json::to_writer(&mut file, &serialized)
            .map_err(|error| format!("Failed to write transcript revision: {error}"))?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Failed to finalize transcript revision staging: {error}"))?;

        publication_barrier(TranscriptRevisionPublicationBarrier::BeforePublication);
        transcript_receipt.revalidate()?;
        let published =
            recording::publish_no_replace(&staging, &path, &file, "publish transcript revision")?;
        drop(published);

        publication_barrier(TranscriptRevisionPublicationBarrier::AfterPublication);
        // A replacement after this check is external post-completion tamper; consumers still
        // hash-check the transcript against this immutable revision before selecting it.
        transcript_receipt.revalidate()
    })();
    if result.is_err() {
        recording::remove_owned_staging(&staging, &file, "publish transcript revision");
    }
    drop(file);
    result
}

fn create_unique_transcript_revision_staging(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    revision: u64,
) -> Result<(std::path::PathBuf, std::fs::File), String> {
    for nonce in 0..128_u64 {
        let path = dir.join(format!(
            "live-{session_id}.transcript.r{revision}.json.part-{}-{nonce}",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to create transcript revision staging file: {error}"
                ));
            }
        }
    }
    Err("Failed to allocate a unique transcript revision staging file".into())
}

fn has_valid_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    capture_sidecar_sha256: &str,
) -> bool {
    let text_name = format!("live-{session_id}.txt");
    let revision_prefix = format!("live-{session_id}.transcript.r");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return false;
        };
        let Some(revision) = name
            .strip_prefix(&revision_prefix)
            .and_then(|value| value.strip_suffix(".json"))
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return false;
        };
        revision > 0
            && transcript_revision_matches_receipt(
                dir,
                &name,
                session_id,
                revision,
                &text_name,
                capture_sidecar_sha256,
            )
    })
}

fn transcript_revision_matches_receipt(
    dir: &std::path::Path,
    revision_name: &str,
    session_id: &crate::audio::session::SessionId,
    revision: u64,
    text_name: &str,
    capture_sidecar_sha256: &str,
) -> bool {
    let Ok((revision_text, _)) = recording::read_and_hash_regular_artifact(dir, revision_name)
    else {
        return false;
    };
    let Ok(_revision) = serde_json::from_str::<TranscriptResultRevision>(&revision_text) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&revision_text) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some(revision_text_name) = object.get("textFile").and_then(|value| value.as_str()) else {
        return false;
    };
    let Some(revision_text_sha256) = object.get("textSha256").and_then(|value| value.as_str())
    else {
        return false;
    };
    let Some(revision_sidecar_sha256) = object
        .get("captureSidecarSha256")
        .and_then(|value| value.as_str())
    else {
        return false;
    };
    let Some(revision_session_id) = object.get("sessionId").and_then(|value| value.as_str()) else {
        return false;
    };
    let Some(revision_number) = object.get("revision").and_then(|value| value.as_u64()) else {
        return false;
    };
    let Ok((_, current_text_sha256)) = recording::read_and_hash_regular_artifact(dir, text_name)
    else {
        return false;
    };
    revision_text_name == text_name
        && revision_sidecar_sha256 == capture_sidecar_sha256
        && revision_session_id == session_id.as_str()
        && revision_number == revision
        && revision_text_sha256 == current_text_sha256
}

fn transcript_result_value(
    result: &TranscriptResultRevision,
    capture_sidecar_sha256: &str,
    transcript_receipt: &PublishedTranscriptReceipt,
) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(result)
        .map_err(|error| format!("Failed to serialize transcript revision: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "Transcript revision did not serialize as an object".to_string())?;
    object.insert("schemaVersion".into(), serde_json::Value::from(1u16));
    object.insert(
        "textFile".into(),
        serde_json::Value::from(transcript_receipt.file_name()),
    );
    object.insert(
        "textSha256".into(),
        serde_json::Value::from(transcript_receipt.sha256()),
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
    stable_path_string(path)
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

fn write_new_text_file(
    path: &std::path::Path,
    text: &str,
) -> Result<PublishedTranscriptReceipt, String> {
    write_new_text_file_with(
        path,
        text,
        |file| file.sync_all(),
        |from, to, owned| recording::publish_no_replace(from, to, owned, "publish live transcript"),
    )
}

fn write_new_text_file_with<S, R>(
    path: &std::path::Path,
    text: &str,
    sync: S,
    rename: R,
) -> Result<PublishedTranscriptReceipt, String>
where
    S: FnOnce(&std::fs::File) -> std::io::Result<()>,
    R: FnOnce(&std::path::Path, &std::path::Path, &std::fs::File) -> Result<std::fs::File, String>,
{
    if path.exists() {
        return Err("live transcript already exists".into());
    }
    let partial = partial_text_path(path).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|error| error.to_string())?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|_| sync(&file))
        .map_err(|error| error.to_string());
    let result = result.and_then(|_| {
        if path.exists() {
            return Err("live transcript already exists".into());
        }
        rename(&partial, path, &file)
    });
    let published = match result {
        Ok(published) => published,
        Err(error) => {
            recording::remove_owned_staging(&partial, &file, "publish live transcript");
            return Err(error);
        }
    };
    drop(file);
    PublishedTranscriptReceipt::from_verified_destination(path, published)
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
    fn history_rejects_linked_legacy_transcripts_inside_or_outside_the_directory_when_supported() {
        let dir = test_dir("history-linked-legacy-transcript");
        let outside = std::env::temp_dir().join(format!(
            "yap-linked-transcript-target-{}",
            std::process::id()
        ));
        std::fs::remove_file(&outside).ok();
        std::fs::write(&outside, "outside\n").unwrap();
        let legacy = dir.join("live-401.txt");
        if let Err(error) = create_file_symlink_for_test(&outside, &legacy) {
            skip_link_test_or_panic(error);
            std::fs::remove_file(&outside).ok();
            std::fs::remove_dir_all(dir).ok();
            return;
        }

        assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
        std::fs::remove_file(&legacy).ok();
        std::fs::remove_file(&outside).ok();

        let inside = dir.join("ordinary-transcript.txt");
        std::fs::write(&inside, "inside\n").unwrap();
        let internal_link = dir.join("live-402.txt");
        create_file_symlink_for_test(&inside, &internal_link).unwrap();
        assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
        std::fs::remove_file(&internal_link).ok();
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn history_ignores_linked_legacy_audio_and_keeps_the_safe_transcript_path() {
        let dir = test_dir("history-linked-legacy-audio");
        let outside =
            std::env::temp_dir().join(format!("yap-linked-audio-target-{}", std::process::id()));
        std::fs::remove_file(&outside).ok();
        std::fs::write(&outside, b"RIFF").unwrap();
        let transcript = dir.join("live-402.txt");
        let audio = dir.join("live-402.wav");
        std::fs::write(&transcript, "safe\n").unwrap();
        if let Err(error) = create_file_symlink_for_test(&outside, &audio) {
            skip_link_test_or_panic(error);
            std::fs::remove_file(&outside).ok();
            std::fs::remove_dir_all(dir).ok();
            return;
        }

        let sessions = list_session_files_from_dir(&dir).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].source_path, transcript.display().to_string());
        assert_eq!(sessions[0].output_path, transcript.display().to_string());
        std::fs::remove_file(&audio).ok();
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn committed_history_falls_back_to_audio_when_its_transcript_is_linked() {
        let dir = test_dir("history-linked-committed-transcript");
        let session = SessionId::new("s-linked-committed-transcript").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        save_finalized_capture_to_dir(&dir, &live_view(Some("hello"), None), Some(capture))
            .unwrap();
        let outside = std::env::temp_dir().join(format!(
            "yap-linked-committed-transcript-target-{}",
            std::process::id()
        ));
        std::fs::remove_file(&outside).ok();
        std::fs::write(&outside, "outside\n").unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));
        std::fs::remove_file(&transcript).unwrap();
        if let Err(error) = create_file_symlink_for_test(&outside, &transcript) {
            skip_link_test_or_panic(error);
            std::fs::remove_file(&outside).ok();
            std::fs::remove_dir_all(dir).ok();
            return;
        }

        let sessions = list_session_files_from_dir(&dir).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].source_path.ends_with(".wav"));
        assert_eq!(sessions[0].output_path, sessions[0].source_path);
        std::fs::remove_file(&transcript).ok();
        std::fs::remove_file(&outside).ok();

        let inside = dir.join("ordinary-committed-transcript.txt");
        std::fs::write(&inside, "inside\n").unwrap();
        create_file_symlink_for_test(&inside, &transcript).unwrap();
        let sessions = list_session_files_from_dir(&dir).unwrap();
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].source_path.ends_with(".wav"));
        assert_eq!(sessions[0].output_path, sessions[0].source_path);
        std::fs::remove_file(&transcript).ok();
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_revision_rejects_a_linked_prior_revision_when_supported() {
        let dir = test_dir("linked-transcript-revision");
        let session = SessionId::new("s-linked-transcript-revision").unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));
        let transcript_receipt = write_new_text_file(&transcript, "first\n").unwrap();
        write_transcript_revision(
            &dir,
            &session,
            &"a".repeat(64),
            &transcript_receipt,
            "first",
            ResultStatus::Complete,
        )
        .unwrap();
        let outside =
            std::env::temp_dir().join(format!("yap-linked-revision-target-{}", std::process::id()));
        std::fs::remove_file(&outside).ok();
        std::fs::write(&outside, "outside revision\n").unwrap();
        let first = transcript_revision_path(&dir, &session, 1);
        std::fs::remove_file(&first).unwrap();
        if let Err(error) = create_file_symlink_for_test(&outside, &first) {
            skip_link_test_or_panic(error);
            std::fs::remove_file(&outside).ok();
            std::fs::remove_dir_all(dir).ok();
            return;
        }

        assert!(write_transcript_revision(
            &dir,
            &session,
            &"a".repeat(64),
            &transcript_receipt,
            "second",
            ResultStatus::Complete,
        )
        .is_err());
        assert!(!transcript_revision_path(&dir, &session, 2).exists());
        std::fs::remove_file(&first).ok();
        std::fs::remove_file(&outside).ok();
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
    fn worker_panic_still_publishes_a_usable_transcript_without_fabricating_history() {
        assert_unavailable_recording_transcript("s-worker-panic", true);
    }

    #[test]
    fn unavailable_worker_still_publishes_a_usable_transcript_without_fabricating_history() {
        assert_unavailable_recording_transcript("s-worker-unavailable", false);
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
            |_, _, _| {
                renamed.set(true);
                Err("test publisher should not be called".into())
            },
        )
        .unwrap_err();

        assert!(error.contains("injected transcript sync failure"));
        assert!(!renamed.get());
        assert!(!transcript.exists());
        assert!(!partial_text_path(&transcript).unwrap().exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_pre_link_replacement_keeps_the_attacker_staging_file_and_writes_no_revision() {
        let dir = test_dir("transcript-pre-link-replacement");
        let session = SessionId::new("s-transcript-pre-link-replacement").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));
        let partial = partial_text_path(&transcript).unwrap();

        let error = save_finalized_capture_to_dir_with_text_publisher(
            &dir,
            &live_view(Some("owned transcript"), None),
            Some(capture),
            |source, destination, owned| {
                let displaced = source.with_extension("displaced");
                std::fs::rename(source, &displaced).map_err(|error| error.to_string())?;
                std::fs::write(source, b"attacker staging").map_err(|error| error.to_string())?;
                recording::publish_no_replace(source, destination, owned, "publish live transcript")
            },
        )
        .unwrap_err();

        assert!(error.contains("staging path no longer names the owned file"));
        assert_eq!(std::fs::read(&partial).unwrap(), b"attacker staging");
        assert!(!transcript.exists());
        assert!(!transcript_revision_path(&dir, &session, 1).exists());
        assert_eq!(recording::scan_recordings(&dir).unwrap().complete.len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_post_link_replacement_keeps_the_attacker_text_and_writes_no_revision() {
        let dir = test_dir("transcript-post-link-replacement");
        let session = SessionId::new("s-transcript-post-link-replacement").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));

        let error = save_finalized_capture_to_dir_with_text_publisher(
            &dir,
            &live_view(Some("owned transcript"), None),
            Some(capture),
            |source, destination, owned| {
                recording::publish_no_replace_with_after_link_for_test(
                    source,
                    destination,
                    owned,
                    "publish live transcript",
                    || {
                        let displaced = destination.with_extension("displaced");
                        std::fs::rename(destination, displaced).unwrap();
                        std::fs::write(destination, b"attacker text").unwrap();
                    },
                )
            },
        )
        .unwrap_err();

        assert!(error.contains("published destination does not name the owned file"));
        assert_eq!(std::fs::read(&transcript).unwrap(), b"attacker text");
        assert!(!transcript_revision_path(&dir, &session, 1).exists());
        assert_eq!(recording::scan_recordings(&dir).unwrap().complete.len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_replacement_after_publication_preserves_independent_text_without_a_revision() {
        let dir = test_dir("transcript-post-publication-replacement");
        let session = SessionId::new("s-transcript-post-publication-replacement").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));

        let saved = save_finalized_capture_to_dir_with_text_publisher(
            &dir,
            &live_view(Some("owned transcript"), None),
            Some(capture),
            |source, destination, owned| {
                let published = recording::publish_no_replace(
                    source,
                    destination,
                    owned,
                    "publish live transcript",
                )?;
                let displaced = destination.with_extension("displaced");
                std::fs::rename(destination, displaced).map_err(|error| error.to_string())?;
                std::fs::write(destination, b"attacker transcript")
                    .map_err(|error| error.to_string())?;
                Ok(published)
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(std::fs::read(&transcript).unwrap(), b"attacker transcript");
        assert!(saved
            .warning
            .as_deref()
            .unwrap_or_default()
            .contains("Transcript revision was not saved"));
        assert!(!transcript_revision_path(&dir, &session, 1).exists());
        let scan = recording::scan_recordings(&dir).unwrap();
        assert_eq!(scan.complete.len(), 1);
        assert_eq!(scan.complete[0].manifest.session_id, session);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_receipt_drops_its_destination_handle_after_save() {
        let dir = test_dir("transcript-receipt-handle-lifetime");
        let session = SessionId::new("s-transcript-receipt-handle-lifetime").unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));

        let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

        assert_eq!(recording::receipt_handle_count_for_test(), 0);
        receipt.revalidate().unwrap();
        assert_eq!(recording::receipt_handle_count_for_test(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_replacement_before_revision_publication_writes_no_revision() {
        let dir = test_dir("transcript-revision-pre-publication-replacement");
        let session = SessionId::new("s-transcript-revision-pre-publication").unwrap();
        let mut recording_capture = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording_capture.append_pcm16(&[1, 0]).unwrap();
        let manifest = recording_capture
            .finalize()
            .unwrap()
            .committed
            .unwrap()
            .manifest;
        let transcript = dir.join(format!("live-{session}.txt"));
        let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

        let error = write_transcript_revision_with_barrier(
            &dir,
            &session,
            &manifest.capture_sidecar_sha256,
            &receipt,
            "owned transcript",
            ResultStatus::Complete,
            |barrier| {
                if barrier == TranscriptRevisionPublicationBarrier::BeforePublication {
                    let displaced = transcript.with_extension("displaced");
                    std::fs::rename(&transcript, displaced).unwrap();
                    std::fs::write(&transcript, "replacement transcript\n").unwrap();
                }
            },
        )
        .unwrap_err();

        assert!(error.contains("transcript path no longer names"));
        assert!(!transcript_revision_path(&dir, &session, 1).exists());
        assert_eq!(
            std::fs::read_to_string(&transcript).unwrap(),
            "replacement transcript\n"
        );
        assert_eq!(recording::receipt_handle_count_for_test(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transcript_replacement_after_revision_publication_is_not_selected_by_history() {
        let dir = test_dir("transcript-revision-post-publication-replacement");
        let session = SessionId::new("s-transcript-revision-post-publication").unwrap();
        let mut recording_capture = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording_capture.append_pcm16(&[1, 0]).unwrap();
        let manifest = recording_capture
            .finalize()
            .unwrap()
            .committed
            .unwrap()
            .manifest;
        let transcript = dir.join(format!("live-{session}.txt"));
        let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

        let error = write_transcript_revision_with_barrier(
            &dir,
            &session,
            &manifest.capture_sidecar_sha256,
            &receipt,
            "owned transcript",
            ResultStatus::Complete,
            |barrier| {
                if barrier == TranscriptRevisionPublicationBarrier::AfterPublication {
                    let displaced = transcript.with_extension("displaced");
                    std::fs::rename(&transcript, displaced).unwrap();
                    std::fs::write(&transcript, "replacement transcript\n").unwrap();
                }
            },
        )
        .unwrap_err();

        assert!(error.contains("transcript path no longer names"));
        assert!(transcript_revision_path(&dir, &session, 1).is_file());
        assert!(!has_valid_transcript_revision(
            &dir,
            &session,
            &manifest.capture_sidecar_sha256,
        ));
        let sessions = list_session_files_from_dir(&dir).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].output_path, sessions[0].source_path);
        assert_eq!(recording::receipt_handle_count_for_test(), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn replaced_capture_sidecar_preserves_text_but_blocks_transcript_revision() {
        let dir = test_dir("transcript-sidecar-revalidation");
        let session = SessionId::new("s-transcript-sidecar-revalidation").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        let sidecar = dir.join(format!("live-{session}.capture.json"));
        let displaced = sidecar.with_extension("displaced");
        std::fs::rename(&sidecar, displaced).unwrap();
        std::fs::write(&sidecar, b"attacker sidecar").unwrap();

        let saved =
            save_finalized_capture_to_dir(&dir, &live_view(Some("survives"), None), Some(capture))
                .unwrap()
                .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join(format!("live-{session}.txt"))).unwrap(),
            "survives\n"
        );
        assert!(saved
            .warning
            .unwrap()
            .contains("Transcript revision was not saved"));
        assert!(!transcript_revision_path(&dir, &session, 1).exists());
        assert!(recording::scan_recordings(&dir)
            .unwrap()
            .complete
            .is_empty());
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
        let transcript_receipt = write_new_text_file(&text_path, "first\n").unwrap();

        write_transcript_revision(
            &dir,
            &manifest.session_id,
            &manifest.capture_sidecar_sha256,
            &transcript_receipt,
            "first",
            ResultStatus::Complete,
        )
        .unwrap();
        write_transcript_revision(
            &dir,
            &manifest.session_id,
            &manifest.capture_sidecar_sha256,
            &transcript_receipt,
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
        assert_eq!(revision["textSha256"], transcript_receipt.sha256());
        assert_eq!(revision["modelId"], crate::stt::nemotron::MODEL_ID);
        let sessions = list_session_files_from_dir(&dir).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].output_path, text_path.display().to_string());
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

    fn skip_link_test_or_panic(error: std::io::Error) {
        if error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(1314)
        {
            return;
        }
        panic!("failed to create test symlink: {error}");
    }

    #[cfg(unix)]
    fn create_file_symlink_for_test(
        original: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }

    #[cfg(windows)]
    fn create_file_symlink_for_test(
        original: &std::path::Path,
        link: &std::path::Path,
    ) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(original, link)
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
        assert_eq!(
            value["textSha256"],
            recording::sha256_file(&dir.join(format!("live-{session}.txt"))).unwrap()
        );
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

    fn assert_unavailable_recording_transcript(session: &str, panicking: bool) {
        let dir = test_dir(&format!("unavailable-recording-{session}"));
        let runtime = live::runtime::LiveRuntime::new();
        let session_id = SessionId::new(session).unwrap();
        if panicking {
            runtime.install_panicking_recording_for_test(session_id.clone());
        } else {
            runtime.install_unavailable_recording_for_test(session_id.clone());
        }

        let saved = save_session_files_to_dir(&runtime, &live_view(Some("survives"), None), &dir)
            .unwrap()
            .unwrap();
        let transcript = dir.join(format!("live-{session_id}.txt"));

        assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "survives\n");
        assert_eq!(saved.source_path, saved.output_path);
        assert!(saved.warning.unwrap().contains(AUDIO_SAVE_FAILED_WARNING));
        assert!(!transcript_revision_path(&dir, &session_id, 1).exists());
        assert!(recording::scan_recordings(&dir)
            .unwrap()
            .complete
            .is_empty());
        assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
        assert!(
            runtime.finalize_recording().is_err(),
            "terminal error remains cached"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
