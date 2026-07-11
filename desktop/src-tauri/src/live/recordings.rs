use std::collections::{HashMap, HashSet};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::evidence::ModelRevision;
use crate::audio::recording::{self, PublishedTranscriptReceipt, RecordingFinalizeResult};
use crate::audio::results::{ResultAuthority, ResultStatus, TranscriptResultRevision};
use crate::audio::session::{SessionMode, SessionOrigin};
use crate::{file_actions, live};

const AUDIO_SAVE_FAILED_WARNING: &str = "Live audio could not be saved. Transcript was saved.";
const TRANSCRIPT_DEGRADED_WARNING: &str = "Live transcript may be incomplete. Audio was saved.";
const PARTIAL_RECOVERY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DELETION_INTENT_SCHEMA_VERSION: u16 = 1;
const MAX_DELETION_ARTIFACTS: usize = 128;
const MAX_MAINTENANCE_WARNINGS: usize = 8;
const MAX_PRIVATE_DELETION_LEFTOVERS: usize = 128;
const PRIVATE_DELETION_LEFTOVER_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletionArtifact {
    name: String,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletionIntent {
    schema_version: u16,
    session_id: crate::audio::session::SessionId,
    reason: String,
    commit_file: String,
    commit_sha256: String,
    artifacts: Vec<DeletionArtifact>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLiveSession {
    pub name: String,
    pub source_path: String,
    pub output_path: String,
    pub created_at_ms: u64,
    pub warning: Option<String>,
    pub capture_commit_path: Option<String>,
    pub recovery_state: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLiveSessionCatalog {
    pub sessions: Vec<SavedLiveSession>,
    pub maintenance_warnings: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableLiveSession {
    pub session_id: String,
    pub name: String,
    pub audio_partial_path: Option<String>,
    pub journal_partial_path: Option<String>,
    pub reason: String,
    pub expires_at_ms: u64,
}

pub fn save_session_files(
    live_runtime: &live::runtime::LiveRuntime,
    view: &live::state::LiveSessionView,
) -> Result<Option<SavedLiveSession>, String> {
    save_session_files_to_dir(live_runtime, view, &recordings_dir())
}

pub(crate) fn save_stop_result(
    stop: &live::runtime::LiveStopResult,
    view: &live::state::LiveSessionView,
) -> Result<Option<SavedLiveSession>, String> {
    match &stop.recording {
        Ok(capture) => save_finalized_capture_to_dir(&recordings_dir(), view, capture.clone()),
        Err(error) => Err(error.clone()),
    }
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
        capture_commit_path: None,
        recovery_state: None,
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

#[cfg(test)]
pub(crate) fn save_finalized_capture_to_dir_for_test(
    dir: &std::path::Path,
    text: &str,
    capture: RecordingFinalizeResult,
) -> Result<(), String> {
    let view = live::state::LiveSessionView {
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
        partial_text: None,
        final_text: Some(text.to_string()),
        transcription_degraded: false,
        error: None,
    };
    save_finalized_capture_to_dir(dir, &view, Some(capture)).map(|_| ())
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
            capture_commit_path: None,
            recovery_state: None,
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
        capture_commit_path: Some(stable_existing_path_string(
            &dir.join(format!("live-{}.commit.json", capture.session_id)),
        )),
        recovery_state: None,
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
    Ok(list_session_catalog()?.sessions)
}

pub fn list_session_catalog() -> Result<SavedLiveSessionCatalog, String> {
    list_session_catalog_from_dir(&recordings_dir())
}

pub fn list_recoverable_live_sessions() -> Result<Vec<RecoverableLiveSession>, String> {
    list_recoverable_live_sessions_from_dir(&recordings_dir())
}

pub fn recover_live_session(session_id: String) -> Result<SavedLiveSession, String> {
    recover_live_session_in_dir(&recordings_dir(), session_id)
}

pub fn delete_recoverable_live_session(session_id: String) -> Result<(), String> {
    delete_recoverable_live_session_in_dir(&recordings_dir(), session_id)
}

pub fn delete_saved_live_session(session_id: String) -> Result<(), String> {
    delete_saved_live_session_in_dir(&recordings_dir(), session_id)
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

#[cfg(test)]
fn list_session_files_from_dir(dir: &std::path::Path) -> Result<Vec<SavedLiveSession>, String> {
    Ok(list_session_catalog_from_dir_at(dir, OffsetDateTime::now_utc())?.sessions)
}

#[cfg(test)]
fn list_session_files_from_dir_at(
    dir: &std::path::Path,
    now: OffsetDateTime,
) -> Result<Vec<SavedLiveSession>, String> {
    Ok(list_session_catalog_from_dir_at(dir, now)?.sessions)
}

fn list_session_catalog_from_dir(dir: &std::path::Path) -> Result<SavedLiveSessionCatalog, String> {
    list_session_catalog_from_dir_at(dir, OffsetDateTime::now_utc())
}

fn list_session_catalog_from_dir_at(
    dir: &std::path::Path,
    now: OffsetDateTime,
) -> Result<SavedLiveSessionCatalog, String> {
    if !dir.exists() {
        return Ok(SavedLiveSessionCatalog {
            sessions: Vec::new(),
            maintenance_warnings: Vec::new(),
        });
    }

    let pending = reconcile_pending_deletion_intents(dir);
    let scan = recording::scan_recordings(dir)?;
    let recoverable = list_recoverable_live_sessions_from_scan(dir, &scan, now)?;
    let maintenance_warnings = damaged_commit_warnings(&scan, pending.maintenance_warnings);
    let (retention_deleted, retention_warnings) =
        reconcile_expired_committed_meetings(dir, &scan.complete, now);
    let mut sessions = scan
        .complete
        .into_iter()
        .filter(|committed| !retention_deleted.contains(committed.manifest.session_id.as_str()))
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
                warning: pending
                    .session_warnings
                    .get(committed.manifest.session_id.as_str())
                    .cloned()
                    .or_else(|| {
                        retention_warnings
                            .get(committed.manifest.session_id.as_str())
                            .cloned()
                    }),
                capture_commit_path: Some(stable_existing_path_string(&dir.join(format!(
                    "live-{}.commit.json",
                    committed.manifest.session_id
                )))),
                recovery_state: None,
            }
        })
        .collect::<Vec<_>>();
    for recoverable in recoverable {
        let session_id = crate::audio::session::SessionId::new(recoverable.session_id.clone())?;
        if let Some(saved) = saved_recovered_session(dir, &session_id)? {
            sessions.push(saved);
        }
    }

    sessions.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(SavedLiveSessionCatalog {
        sessions,
        maintenance_warnings,
    })
}

fn reconcile_expired_committed_meetings(
    dir: &Path,
    committed: &[recording::CommittedCapture],
    now: OffsetDateTime,
) -> (HashSet<String>, HashMap<String, String>) {
    let mut deleted = HashSet::new();
    let mut warnings = HashMap::new();
    for capture in committed {
        if !committed_meeting_is_expired(&capture.manifest, now) {
            continue;
        }
        match delete_expired_committed_meeting(dir, capture) {
            Ok(()) => {
                deleted.insert(capture.manifest.session_id.to_string());
            }
            Err(error) => {
                warnings.insert(
                    capture.manifest.session_id.to_string(),
                    format!("Expired meeting cleanup is pending: {error}"),
                );
            }
        }
    }
    (deleted, warnings)
}

fn committed_meeting_is_expired(
    manifest: &recording::CaptureCommitManifest,
    now: OffsetDateTime,
) -> bool {
    let Some(metadata) = &manifest.session_metadata else {
        return false;
    };
    if metadata.session_id != manifest.session_id
        || metadata.mode != SessionMode::Meeting
        || metadata.origin != SessionOrigin::LiveCapture
    {
        return false;
    }
    let Some(expiry) = metadata.retention_expires_at_utc.as_deref() else {
        return false;
    };
    OffsetDateTime::parse(expiry, &Rfc3339)
        .map(|expiry| expiry <= now)
        .unwrap_or(false)
}

fn delete_expired_committed_meeting(
    dir: &Path,
    capture: &recording::CommittedCapture,
) -> Result<(), String> {
    delete_committed_session_in_dir(dir, capture, "expired-meeting-retention")
}

fn delete_saved_live_session_in_dir(dir: &Path, session_id: String) -> Result<(), String> {
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    let scan = recording::scan_recordings(dir)?;
    let capture = scan
        .complete
        .iter()
        .find(|capture| capture.manifest.session_id == session_id)
        .ok_or_else(|| "Live recording is not a hash-valid committed Yap session.".to_string())?;
    delete_committed_session_in_dir(dir, capture, "manual")
}

fn delete_committed_session_in_dir(
    dir: &Path,
    capture: &recording::CommittedCapture,
    reason: &str,
) -> Result<(), String> {
    let intent = build_deletion_intent(dir, capture, reason)?;
    let intent_name = deletion_intent_name(&intent.session_id);
    let intent_path = dir.join(&intent_name);
    write_deletion_intent(&intent_path, &intent)?;
    resume_deletion_intent(dir, &intent_name)
}

fn build_deletion_intent(
    dir: &Path,
    capture: &recording::CommittedCapture,
    reason: &str,
) -> Result<DeletionIntent, String> {
    if reason != "manual" && reason != "expired-meeting-retention" {
        return Err("unsupported recording deletion reason".into());
    }
    let manifest = &capture.manifest;
    let commit_file = format!("live-{}.commit.json", manifest.session_id);
    let (commit_text, commit_sha256) =
        recording::read_and_hash_regular_artifact(dir, &commit_file)?;
    let current_manifest: recording::CaptureCommitManifest = serde_json::from_str(&commit_text)
        .map_err(|error| format!("Failed to parse committed recording manifest: {error}"))?;
    if current_manifest != *manifest {
        return Err("committed recording manifest changed before deletion".into());
    }
    let mut artifacts = vec![
        DeletionArtifact {
            name: manifest.audio_file.clone(),
            sha256: manifest.audio_sha256.clone(),
        },
        DeletionArtifact {
            name: manifest.capture_sidecar_file.clone(),
            sha256: manifest.capture_sidecar_sha256.clone(),
        },
    ];
    let journal = format!("live-{}.capture.journal.part", manifest.session_id);
    if recording::is_regular_artifact(&dir.join(&journal)) {
        let (journal_text, journal_sha256) =
            recording::read_and_hash_regular_artifact(dir, &journal)?;
        let parsed = recording::parse_journal_for_session(&journal_text, &manifest.session_id)?;
        if parsed {
            artifacts.push(DeletionArtifact {
                name: journal,
                sha256: journal_sha256,
            });
        }
    }
    let transcript_names = transcript_artifact_names(dir, &manifest.session_id)?;
    if !transcript_names.is_empty() {
        let highest = highest_transcript_revision(dir, &manifest.session_id).ok_or_else(|| {
            "transcript artifacts do not contain a numbered immutable revision".to_string()
        })?;
        let expected = std::iter::once(format!("live-{}.txt", manifest.session_id))
            .chain((1..=highest).map(|revision| {
                format!("live-{}.transcript.r{revision}.json", manifest.session_id)
            }))
            .collect::<HashSet<_>>();
        if transcript_names != expected
            || !has_valid_transcript_revision(
                dir,
                &manifest.session_id,
                &manifest.capture_sidecar_sha256,
            )
        {
            return Err("transcript artifacts are incomplete or do not form a valid immutable revision chain".into());
        }
        for name in transcript_names {
            artifacts.push(DeletionArtifact {
                sha256: recording::sha256_regular_artifact(dir, &name)?,
                name,
            });
        }
        let polished = format!("live-{}.polished.txt", manifest.session_id);
        if recording::is_regular_artifact(&dir.join(&polished)) {
            artifacts.push(DeletionArtifact {
                sha256: recording::sha256_regular_artifact(dir, &polished)?,
                name: polished,
            });
        }
    }
    let intent = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: manifest.session_id.clone(),
        reason: reason.to_string(),
        commit_file,
        commit_sha256,
        artifacts,
    };
    validate_deletion_intent(&intent)?;
    Ok(intent)
}

fn deletion_intent_name(session_id: &crate::audio::session::SessionId) -> String {
    format!("live-{session_id}.deletion.v1.json")
}

fn write_deletion_intent(path: &Path, intent: &DeletionIntent) -> Result<(), String> {
    write_deletion_intent_with_publication_barrier(path, intent, |_| {})
}

fn write_deletion_intent_with_publication_barrier<F>(
    path: &Path,
    intent: &DeletionIntent,
    mut publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    validate_deletion_intent(intent)?;
    let dir = path
        .parent()
        .ok_or_else(|| "recording deletion intent has no parent directory".to_string())?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "recording deletion intent has no valid file name".to_string())?;
    if name != deletion_intent_name(&intent.session_id) {
        return Err("recording deletion intent name does not match its session".into());
    }
    let replace_corrupt_final = if physical_entry_exists(dir, name)? {
        let existing = recording::read_regular_artifact(dir, name)
            .ok()
            .and_then(|text| {
                serde_json::from_str::<DeletionIntent>(&text)
                    .ok()
                    .filter(|existing| validate_deletion_intent(existing).is_ok())
            });
        if existing.as_ref() == Some(intent) {
            return Ok(());
        }
        if !intent_originals_are_intact(dir, intent)? {
            return Err("recording deletion intent is corrupt and deletion may have started; evidence was retained".into());
        }
        true
    } else {
        false
    };

    let (staging, mut file) = create_unique_deletion_intent_staging(dir, &intent.session_id)?;
    let mut quarantined = None;
    let result = (|| {
        serde_json::to_writer(&mut file, intent)
            .map_err(|error| format!("Failed to serialize recording deletion intent: {error}"))?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Failed to persist recording deletion intent: {error}"))?;
        if replace_corrupt_final {
            quarantined = Some(recording::quarantine_regular_artifact(dir, name)?);
        }
        publication_barrier(false);
        let published = recording::publish_no_replace(
            &staging,
            path,
            &file,
            "publish recording deletion intent",
        )?;
        drop(published);
        publication_barrier(true);
        let published = recording::read_regular_artifact(dir, name)?;
        let published: DeletionIntent = serde_json::from_str(&published).map_err(|error| {
            format!("Failed to re-read published recording deletion intent: {error}")
        })?;
        if published != *intent {
            return Err("published recording deletion intent changed before verification".into());
        }
        if let Some(quarantined) = quarantined.as_ref() {
            recording::remove_verified_quarantined_artifact(quarantined)?;
        }
        let _ = recording::sync_recordings_parent(dir);
        Ok(())
    })();
    if let Err(error) = &result {
        if let Some(quarantined) = quarantined.as_ref() {
            if let Err(restore_error) =
                recording::restore_verified_quarantined_artifact(quarantined, path)
            {
                crate::stt::log_yap(&format!(
                    "Retained quarantined recording deletion intent after publication failure: {restore_error}"
                ));
            }
        }
        recording::remove_owned_staging(&staging, &file, "publish recording deletion intent");
        crate::stt::log_yap(&format!(
            "Failed to publish recording deletion intent: {error}"
        ));
    }
    drop(file);
    result
}

fn create_unique_deletion_intent_staging(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<(std::path::PathBuf, std::fs::File), String> {
    for nonce in 0..128_u64 {
        let path = dir.join(format!(
            ".live-{session_id}.deletion.v1.{}-{nonce}.part",
            std::process::id()
        ));
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to create recording deletion intent staging file: {error}"
                ))
            }
        }
    }
    Err("Failed to allocate a private recording deletion intent staging file".into())
}

struct ReconciliationWarnings {
    session_warnings: HashMap<String, String>,
    maintenance_warnings: Vec<String>,
}

fn reconcile_pending_deletion_intents(dir: &Path) -> ReconciliationWarnings {
    let mut warnings = ReconciliationWarnings {
        session_warnings: HashMap::new(),
        maintenance_warnings: Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return warnings;
    };
    let mut names = Vec::new();
    for entry in entries {
        if names.len() == MAX_PRIVATE_DELETION_LEFTOVERS {
            push_maintenance_warning(
                &mut warnings.maintenance_warnings,
                "Private recording deletion cleanup scan reached its fixed budget.".into(),
            );
            break;
        }
        let Ok(entry) = entry else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        names.push(name);
    }
    reconcile_private_deletion_leftovers(dir, &names, &mut warnings.maintenance_warnings);
    for name in names {
        if !name.starts_with("live-") || !name.ends_with(".deletion.v1.json") {
            continue;
        }
        if let Err(error) = resume_deletion_intent(dir, &name) {
            let session = name
                .trim_start_matches("live-")
                .trim_end_matches(".deletion.v1.json");
            if crate::audio::session::SessionId::new(session.to_string()).is_ok() {
                let warning = format!("Recording deletion is pending: {error}");
                warnings
                    .session_warnings
                    .insert(session.to_string(), warning.clone());
                push_maintenance_warning(&mut warnings.maintenance_warnings, warning);
            } else {
                push_maintenance_warning(
                    &mut warnings.maintenance_warnings,
                    format!("Recording cleanup evidence was retained: {name}: {error}"),
                );
            }
        }
    }
    warnings
}

#[derive(Clone, Copy)]
enum PrivateDeletionLeftover {
    Staging { process_id: u32 },
    Quarantine,
}

fn reconcile_private_deletion_leftovers(dir: &Path, names: &[String], warnings: &mut Vec<String>) {
    for name in names {
        match private_deletion_leftover(name) {
            Some(PrivateDeletionLeftover::Staging { process_id })
                if process_id == std::process::id() => {}
            Some(_) => {
                match private_deletion_leftover_is_old(dir, name) {
                    Ok(true) => {
                        if let Err(error) = recording::remove_regular_artifact(dir, name) {
                            push_maintenance_warning(
                            warnings,
                            format!("Private recording deletion cleanup was retained: {name}: {error}"),
                        );
                        }
                    }
                    Ok(false) => {}
                    Err(error) => push_maintenance_warning(
                        warnings,
                        format!("Private recording deletion cleanup was retained: {name}: {error}"),
                    ),
                }
            }
            None if looks_like_private_deletion_artifact(name) => push_maintenance_warning(
                warnings,
                format!("Unknown private deletion artifact was retained: {name}"),
            ),
            None => {}
        }
    }
}

fn private_deletion_leftover(name: &str) -> Option<PrivateDeletionLeftover> {
    let stem = name.strip_prefix(".live-")?;
    if let Some((session, suffix)) = stem
        .strip_suffix(".part")
        .and_then(|value| value.split_once(".deletion.v1."))
    {
        crate::audio::session::SessionId::new(session.to_string()).ok()?;
        let (process_id, nonce) = suffix.split_once('-')?;
        process_id.parse::<u32>().ok()?;
        nonce.parse::<u64>().ok()?;
        return Some(PrivateDeletionLeftover::Staging {
            process_id: process_id.parse().ok()?,
        });
    }
    let (session, suffix) = stem.split_once(".deletion.v1.json.delete-")?;
    crate::audio::session::SessionId::new(session.to_string()).ok()?;
    let (process_id, nonce) = suffix.split_once('-')?;
    process_id.parse::<u32>().ok()?;
    nonce.parse::<u64>().ok()?;
    Some(PrivateDeletionLeftover::Quarantine)
}

fn looks_like_private_deletion_artifact(name: &str) -> bool {
    name.starts_with(".live-") && name.contains(".deletion.v1.")
}

fn private_deletion_leftover_is_old(dir: &Path, name: &str) -> Result<bool, String> {
    let file = recording::open_regular_artifact(dir, name)?;
    let modified = file
        .metadata()
        .map_err(|error| format!("Failed to inspect private deletion artifact: {error}"))?
        .modified()
        .map_err(|error| format!("Failed to inspect private deletion artifact age: {error}"))?;
    let modified = system_time_to_unix_millis(modified)
        .ok_or_else(|| "Private deletion artifact has an invalid modification time".to_string())?;
    let now = unix_millis_now()?;
    Ok(now.saturating_sub(modified) >= PRIVATE_DELETION_LEFTOVER_TTL.as_millis() as u64)
}

fn push_maintenance_warning(warnings: &mut Vec<String>, warning: String) {
    if warnings.len() < MAX_MAINTENANCE_WARNINGS && !warnings.contains(&warning) {
        warnings.push(warning);
    }
}

fn resume_deletion_intent(dir: &Path, intent_name: &str) -> Result<(), String> {
    let (text, intent_sha256) = recording::read_and_hash_regular_artifact(dir, intent_name)?;
    let intent: DeletionIntent = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse recording deletion intent: {error}"))?;
    validate_deletion_intent(&intent)?;
    if intent_name != deletion_intent_name(&intent.session_id) {
        return Err("recording deletion intent name does not match its session".into());
    }
    if physical_entry_exists(dir, &intent.commit_file)? {
        prove_intent_against_current_commit(dir, &intent, OffsetDateTime::now_utc())?;
        for artifact in &intent.artifacts {
            remove_regular_artifact_if_present(dir, &artifact.name, &artifact.sha256)?;
        }
        recording::remove_regular_artifact_if_hash(
            dir,
            &intent.commit_file,
            &intent.commit_sha256,
        )?;
    } else {
        ensure_intent_artifacts_are_absent(dir, &intent)?;
    }
    recording::remove_regular_artifact_if_hash(dir, intent_name, &intent_sha256)
}

fn prove_intent_against_current_commit(
    dir: &Path,
    intent: &DeletionIntent,
    now: OffsetDateTime,
) -> Result<(), String> {
    let (commit_text, commit_sha256) =
        recording::read_and_hash_regular_artifact(dir, &intent.commit_file)?;
    if commit_sha256 != intent.commit_sha256 {
        return Err("recording deletion commit no longer matches the published intent".into());
    }
    let manifest: recording::CaptureCommitManifest = serde_json::from_str(&commit_text)
        .map_err(|error| format!("Failed to parse committed recording manifest: {error}"))?;
    manifest.validate()?;
    if manifest.session_id != intent.session_id
        || !intent_has_artifact(intent, &manifest.audio_file, &manifest.audio_sha256)
        || !intent_has_artifact(
            intent,
            &manifest.capture_sidecar_file,
            &manifest.capture_sidecar_sha256,
        )
    {
        return Err(
            "recording deletion intent does not match the current committed session".into(),
        );
    }
    if intent.reason == "expired-meeting-retention" && !committed_meeting_is_expired(&manifest, now)
    {
        return Err(
            "expired-meeting deletion is no longer authorized by the committed metadata".into(),
        );
    }
    Ok(())
}

fn intent_has_artifact(intent: &DeletionIntent, name: &str, sha256: &str) -> bool {
    intent
        .artifacts
        .iter()
        .any(|artifact| artifact.name == name && artifact.sha256 == sha256)
}

fn intent_originals_are_intact(dir: &Path, intent: &DeletionIntent) -> Result<bool, String> {
    if !physical_entry_exists(dir, &intent.commit_file)? {
        return Ok(false);
    }
    if prove_intent_against_current_commit(dir, intent, OffsetDateTime::now_utc()).is_err() {
        return Ok(false);
    }
    for artifact in &intent.artifacts {
        if !physical_entry_exists(dir, &artifact.name)?
            || recording::sha256_regular_artifact(dir, &artifact.name)? != artifact.sha256
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn remove_regular_artifact_if_present(dir: &Path, name: &str, sha256: &str) -> Result<(), String> {
    if physical_entry_exists(dir, name)? {
        recording::remove_regular_artifact_if_hash(dir, name, sha256)
    } else {
        Ok(())
    }
}

fn ensure_intent_artifacts_are_absent(dir: &Path, intent: &DeletionIntent) -> Result<(), String> {
    for artifact in &intent.artifacts {
        if physical_entry_exists(dir, &artifact.name)? {
            return Err(
                "recording deletion commit is missing but an intended artifact remains".into(),
            );
        }
    }
    Ok(())
}

fn physical_entry_exists(dir: &Path, name: &str) -> Result<bool, String> {
    recording::validate_artifact_name(name)?;
    match std::fs::symlink_metadata(dir.join(name)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Failed to inspect recording deletion artifact: {error}"
        )),
    }
}

fn validate_deletion_intent(intent: &DeletionIntent) -> Result<(), String> {
    if intent.schema_version != DELETION_INTENT_SCHEMA_VERSION
        || (intent.reason != "manual" && intent.reason != "expired-meeting-retention")
        || intent.commit_file != format!("live-{}.commit.json", intent.session_id)
        || intent.artifacts.is_empty()
        || intent.artifacts.len() > MAX_DELETION_ARTIFACTS
        || !is_sha256(&intent.commit_sha256)
    {
        return Err("recording deletion intent has an unsupported shape".into());
    }
    let mut names = HashSet::new();
    for artifact in &intent.artifacts {
        recording::validate_artifact_name(&artifact.name)?;
        if !names.insert(artifact.name.clone())
            || artifact.name == intent.commit_file
            || !is_deletion_artifact_for_session(&artifact.name, &intent.session_id)
        {
            return Err("recording deletion intent names an unowned artifact".into());
        }
        if !is_sha256(&artifact.sha256) {
            return Err("recording deletion intent has an invalid artifact hash".into());
        }
    }
    if !names.contains(&format!("live-{}.wav", intent.session_id))
        || !names.contains(&format!("live-{}.capture.json", intent.session_id))
    {
        return Err("recording deletion intent is missing required capture artifacts".into());
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_deletion_artifact_for_session(
    name: &str,
    session_id: &crate::audio::session::SessionId,
) -> bool {
    let stem = format!("live-{session_id}");
    name == format!("{stem}.wav")
        || name == format!("{stem}.capture.json")
        || name == format!("{stem}.capture.journal.part")
        || name == format!("{stem}.txt")
        || name == format!("{stem}.polished.txt")
        || name
            .strip_prefix(&format!("{stem}.transcript.r"))
            .and_then(|value| value.strip_suffix(".json"))
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|revision| revision > 0)
}

pub(crate) fn canonical_committed_live_path_from_dir(
    requested: &Path,
    owned_dir: &Path,
    require_transcript: bool,
) -> Result<std::path::PathBuf, String> {
    let owned_dir = owned_dir
        .canonicalize()
        .map_err(|_| "Yap recordings directory is unavailable.".to_string())?;
    let path = requested
        .canonicalize()
        .map_err(|_| "Yap recording is unavailable.".to_string())?;
    if path.parent() != Some(owned_dir.as_path()) || !recording::is_regular_artifact(&path) {
        return Err("Yap recording is not a canonical committed session artifact.".into());
    }
    let scan = recording::scan_recordings(&owned_dir)?;
    for capture in scan.complete {
        let session_id = &capture.manifest.session_id;
        let audio = owned_dir.join(&capture.manifest.audio_file);
        let text = owned_dir.join(format!("live-{session_id}.txt"));
        if path == audio && !require_transcript {
            return Ok(path);
        }
        if path == text
            && has_valid_transcript_revision(
                &owned_dir,
                session_id,
                &capture.manifest.capture_sidecar_sha256,
            )
        {
            return Ok(path);
        }
    }
    Err("Yap recording is not a canonical committed session artifact.".into())
}

pub(crate) fn open_committed_live_transcript_from_dir(
    requested: &Path,
    owned_dir: &Path,
) -> Result<std::fs::File, String> {
    let owned_dir = owned_dir
        .canonicalize()
        .map_err(|_| "Yap recordings directory is unavailable.".to_string())?;
    let parent = requested
        .parent()
        .ok_or_else(|| "Yap recording is unavailable.".to_string())?
        .canonicalize()
        .map_err(|_| "Yap recording is unavailable.".to_string())?;
    let name = requested
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Yap recording is unavailable.".to_string())?;
    if parent != owned_dir || !file_actions::is_transcript_path(std::path::Path::new(name)) {
        return Err("Yap recording is not a canonical committed session artifact.".into());
    }
    let mut file = recording::open_regular_artifact(&owned_dir, name)?;
    let handle_sha256 = recording::sha256_open_regular_file(&mut file)?;
    let scan = recording::scan_recordings(&owned_dir)?;
    for capture in scan.complete {
        let session_id = &capture.manifest.session_id;
        if name != format!("live-{session_id}.txt")
            || !has_valid_transcript_revision(
                &owned_dir,
                session_id,
                &capture.manifest.capture_sidecar_sha256,
            )
        {
            continue;
        }
        let Some(expected_sha256) = validated_transcript_sha256(&owned_dir, session_id) else {
            continue;
        };
        if expected_sha256 == handle_sha256 {
            file.seek(SeekFrom::Start(0))
                .map_err(|error| format!("Failed to rewind validated Yap transcript: {error}"))?;
            return Ok(file);
        }
        return Err("Yap transcript changed after validation.".into());
    }
    Err("Yap recording is not a canonical committed session artifact.".into())
}

fn validated_transcript_sha256(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Option<String> {
    let highest = highest_transcript_revision(dir, session_id)?;
    let revision_name = format!("live-{session_id}.transcript.r{highest}.json");
    let (text, _) = recording::read_and_hash_regular_artifact(dir, &revision_name).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("textSha256")?
        .as_str()
        .map(str::to_string)
}

fn list_recoverable_live_sessions_from_dir(
    dir: &Path,
) -> Result<Vec<RecoverableLiveSession>, String> {
    list_recoverable_live_sessions_from_scan(
        dir,
        &recording::scan_recordings(dir)?,
        OffsetDateTime::now_utc(),
    )
}

fn list_recoverable_live_sessions_from_scan(
    dir: &Path,
    scan: &recording::RecordingScan,
    now: OffsetDateTime,
) -> Result<Vec<RecoverableLiveSession>, String> {
    let mut sessions = Vec::new();
    for recovered in &scan.recovered_partial {
        if let Some(saved) = saved_recovered_session(dir, &recovered.session_id)? {
            sessions.push(RecoverableLiveSession {
                session_id: recovered.session_id.to_string(),
                name: saved.name,
                audio_partial_path: Some(saved.source_path),
                journal_partial_path: regular_artifact_exists(
                    dir,
                    &format!("live-{}.capture.journal.part", recovered.session_id),
                )
                .then(|| {
                    stable_existing_path_string(&dir.join(format!(
                        "live-{}.capture.journal.part",
                        recovered.session_id
                    )))
                }),
                reason: "Recovered partial recording is retained for recovery or deletion.".into(),
                expires_at_ms: u64::MAX,
            });
        }
    }
    for partial in &scan.partial {
        let Some(session_id) = partial.session_id.as_ref() else {
            continue;
        };
        let candidate = recoverable_session_from_dir(dir, session_id)?;
        let now_ms = u64::try_from(now.unix_timestamp_nanos() / 1_000_000).unwrap_or(u64::MAX);
        if candidate.expires_at_ms <= now_ms {
            if let Err(error) = delete_recoverable_live_session_in_dir(dir, session_id.to_string())
            {
                sessions.push(RecoverableLiveSession {
                    reason: format!("{} Cleanup is pending: {error}", candidate.reason),
                    ..candidate
                });
            }
            continue;
        }
        sessions.push(candidate);
    }
    Ok(sessions)
}

fn damaged_commit_warnings(
    scan: &recording::RecordingScan,
    mut warnings: Vec<String>,
) -> Vec<String> {
    for damaged in &scan.damaged {
        push_maintenance_warning(
            &mut warnings,
            format!(
                "Damaged live recording {} was preserved: {}",
                damaged.session_id, damaged.reason
            ),
        );
    }
    warnings
}

fn recoverable_session_from_dir(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<RecoverableLiveSession, String> {
    let name = format!("live-{session_id}");
    let audio_name = format!("{name}.wav.part");
    let journal_name = format!("{name}.capture.journal.part");
    let orphan_audio_name = format!("{name}.wav");
    let audio = regular_artifact_exists(dir, &audio_name)
        .then(|| stable_existing_path_string(&dir.join(&audio_name)))
        .or_else(|| {
            regular_artifact_exists(dir, &orphan_audio_name)
                .then(|| stable_existing_path_string(&dir.join(&orphan_audio_name)))
        });
    let journal = regular_artifact_exists(dir, &journal_name)
        .then(|| stable_existing_path_string(&dir.join(&journal_name)));
    let recorded_at = [
        audio_name.as_str(),
        orphan_audio_name.as_str(),
        journal_name.as_str(),
    ]
    .into_iter()
    .filter_map(|artifact| artifact_modified_at_ms(dir, artifact))
    .min()
    .unwrap_or_else(|| unix_millis_now().unwrap_or(0));
    let expires_at_ms = recorded_at.saturating_add(PARTIAL_RECOVERY_TTL.as_millis() as u64);
    Ok(RecoverableLiveSession {
        session_id: session_id.to_string(),
        name,
        audio_partial_path: audio,
        journal_partial_path: journal,
        reason: "Recording did not finish and can be recovered or deleted.".into(),
        expires_at_ms,
    })
}

fn recover_live_session_in_dir(dir: &Path, session_id: String) -> Result<SavedLiveSession, String> {
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    if let Some(saved) = saved_recovered_session(dir, &session_id)? {
        return Ok(saved);
    }
    ensure_recoverable_session(dir, &session_id)?;
    let candidate = recoverable_session_from_dir(dir, &session_id)?;
    if candidate.expires_at_ms <= unix_millis_now()? {
        return Err("Recoverable live recording has expired.".into());
    }
    if candidate.audio_partial_path.is_none() {
        return Err("Recoverable live recording has no safe partial WAV to repair.".into());
    }
    let (audio_file, audio_bytes, audio_sha256) = recording::recover_partial_wav(dir, &session_id)?;
    let sidecar_file = format!("live-{session_id}.capture.partial.json");
    let sidecar_sha256 = match recording::read_and_hash_regular_artifact(dir, &sidecar_file) {
        Ok((text, hash)) if valid_partial_sidecar(&text, &session_id) => hash,
        Ok(_) => return Err("Partial capture sidecar does not match the recovery session.".into()),
        Err(_) => {
            let sidecar = serde_json::json!({
                "schemaVersion": 1u16,
                "sessionId": session_id,
                "status": "partial",
            });
            let receipt = write_new_text_file(
                &dir.join(&sidecar_file),
                &format!(
                    "{}\n",
                    serde_json::to_string(&sidecar).map_err(|error| error.to_string())?
                ),
            )?;
            receipt.sha256().to_string()
        }
    };
    let commit_file = format!("live-{session_id}.commit.json");
    let commit = recording::PartialRecoveryCommit {
        schema_version: 1,
        session_id: session_id.clone(),
        status: recording::CaptureStatus::Partial,
        audio_file: audio_file.clone(),
        audio_sha256,
        audio_bytes,
        capture_sidecar_file: sidecar_file,
        capture_sidecar_sha256: sidecar_sha256,
        committed_at_utc: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|_| "Failed to format recovery timestamp")?,
    };
    write_new_text_file(
        &dir.join(&commit_file),
        &format!(
            "{}\n",
            serde_json::to_string(&commit).map_err(|error| error.to_string())?
        ),
    )?;
    saved_recovered_session(dir, &session_id)?
        .ok_or_else(|| "Recovered live recording was not verifiable.".into())
}

fn delete_recoverable_live_session_in_dir(dir: &Path, session_id: String) -> Result<(), String> {
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    ensure_recoverable_session(dir, &session_id)?;
    let recovered = saved_recovered_session(dir, &session_id)?.is_some();
    for suffix in [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json.part",
        ".commit.json.part",
    ] {
        let name = format!("live-{session_id}{suffix}");
        if regular_artifact_exists(dir, &name) {
            recording::remove_regular_artifact(dir, &name)?;
        }
    }
    let sidecar_name = format!("live-{session_id}.capture.partial.json");
    if let Ok(text) = recording::read_regular_artifact(dir, &sidecar_name) {
        if valid_partial_sidecar(&text, &session_id) {
            recording::remove_regular_artifact(dir, &sidecar_name)?;
        }
    }
    if recovered {
        recording::remove_regular_artifact(dir, &format!("live-{session_id}.commit.json"))?;
    }
    let orphan_audio = format!("live-{session_id}.wav");
    if regular_artifact_exists(dir, &orphan_audio) {
        recording::remove_regular_artifact(dir, &orphan_audio)?;
    }
    Ok(())
}

fn ensure_recoverable_session(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<(), String> {
    let scan = recording::scan_recordings(dir)?;
    (scan
        .partial
        .into_iter()
        .any(|partial| partial.session_id.as_ref() == Some(session_id))
        || scan
            .recovered_partial
            .into_iter()
            .any(|recovered| recovered.session_id == *session_id))
    .then_some(())
    .ok_or_else(|| "Live recording is not a recoverable Yap session.".into())
}

fn saved_recovered_session(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<Option<SavedLiveSession>, String> {
    let commit_name = format!("live-{session_id}.commit.json");
    let Ok((text, _)) = recording::read_and_hash_regular_artifact(dir, &commit_name) else {
        return Ok(None);
    };
    let Ok(commit) = serde_json::from_str::<recording::PartialRecoveryCommit>(&text) else {
        return Ok(None);
    };
    if commit.schema_version != 1
        || commit.status != recording::CaptureStatus::Partial
        || commit.session_id != *session_id
        || !is_expected_recovery_name(&commit.audio_file, session_id, ".wav")
        || !is_expected_recovery_name(
            &commit.capture_sidecar_file,
            session_id,
            ".capture.partial.json",
        )
        || recording::sha256_regular_artifact(dir, &commit.audio_file)? != commit.audio_sha256
        || recording::sha256_regular_artifact(dir, &commit.capture_sidecar_file)?
            != commit.capture_sidecar_sha256
    {
        return Ok(None);
    }
    let audio = recording::open_regular_artifact(dir, &commit.audio_file)?;
    if audio.metadata().map_err(|error| error.to_string())?.len() != commit.audio_bytes {
        return Ok(None);
    }
    let sidecar = recording::read_regular_artifact(dir, &commit.capture_sidecar_file)?;
    if !valid_partial_sidecar(&sidecar, session_id) {
        return Ok(None);
    }
    Ok(Some(SavedLiveSession {
        name: format!("live-{session_id}"),
        source_path: stable_existing_path_string(&dir.join(&commit.audio_file)),
        output_path: stable_existing_path_string(&dir.join(&commit.audio_file)),
        created_at_ms: committed_at_ms(&commit.committed_at_utc),
        warning: Some("Recovered partial recording.".into()),
        capture_commit_path: Some(stable_existing_path_string(&dir.join(commit_name))),
        recovery_state: Some("recoverable".into()),
    }))
}

fn valid_partial_sidecar(text: &str, session_id: &crate::audio::session::SessionId) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
        && value.get("sessionId").and_then(serde_json::Value::as_str) == Some(session_id.as_str())
        && value.get("status").and_then(serde_json::Value::as_str) == Some("partial")
}

fn is_expected_recovery_name(
    name: &str,
    session_id: &crate::audio::session::SessionId,
    suffix: &str,
) -> bool {
    recording::validate_artifact_name(name).is_ok() && name == format!("live-{session_id}{suffix}")
}

fn artifact_modified_at_ms(dir: &Path, name: &str) -> Option<u64> {
    let file = recording::open_regular_artifact(dir, name).ok()?;
    system_time_to_unix_millis(file.metadata().ok()?.modified().ok()?)
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
    let highest = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str().map(str::to_owned)?;
            name.strip_prefix(&revision_prefix)
                .and_then(|value| value.strip_suffix(".json"))
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|revision| *revision > 0)
        })
        .max();
    let Some(highest) = highest else {
        return false;
    };
    transcript_revision_chain_matches_receipt(
        dir,
        session_id,
        highest,
        &text_name,
        capture_sidecar_sha256,
    )
}

fn transcript_revision_chain_matches_receipt(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
    highest_revision: u64,
    text_name: &str,
    capture_sidecar_sha256: &str,
) -> bool {
    let Ok((_, current_text_sha256)) = recording::read_and_hash_regular_artifact(dir, text_name)
    else {
        return false;
    };
    let mut previous_hash = None;
    for revision in 1..=highest_revision {
        let revision_name = format!("live-{session_id}.transcript.r{revision}.json");
        let Ok((revision_text, revision_hash)) =
            recording::read_and_hash_regular_artifact(dir, &revision_name)
        else {
            return false;
        };
        if serde_json::from_str::<TranscriptResultRevision>(&revision_text).is_err() {
            return false;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&revision_text) else {
            return false;
        };
        let Some(object) = value.as_object() else {
            return false;
        };
        let previous_matches = match previous_hash.as_deref() {
            Some(previous_hash) => {
                object
                    .get("previousResultSha256")
                    .and_then(|value| value.as_str())
                    == Some(previous_hash)
            }
            None => {
                object.get("previousResultSha256").is_none()
                    || object
                        .get("previousResultSha256")
                        .is_some_and(serde_json::Value::is_null)
            }
        };
        if object.get("textFile").and_then(|value| value.as_str()) != Some(text_name)
            || object.get("textSha256").and_then(|value| value.as_str())
                != Some(current_text_sha256.as_str())
            || object
                .get("captureSidecarSha256")
                .and_then(|value| value.as_str())
                != Some(capture_sidecar_sha256)
            || object.get("sessionId").and_then(|value| value.as_str()) != Some(session_id.as_str())
            || object.get("revision").and_then(|value| value.as_u64()) != Some(revision)
            || !previous_matches
        {
            return false;
        }
        previous_hash = Some(revision_hash);
    }
    true
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

fn highest_transcript_revision(
    dir: &std::path::Path,
    session_id: &crate::audio::session::SessionId,
) -> Option<u64> {
    let prefix = format!("live-{session_id}.transcript.r");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter_map(|name| {
            name.strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(".json"))
                .and_then(|value| value.parse::<u64>().ok())
        })
        .filter(|revision| *revision > 0)
        .max()
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
            .is_some_and(|stem| stem.starts_with("live-s-"))
}

fn transcript_artifact_names(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
) -> Result<HashSet<String>, String> {
    let text = format!("live-{session_id}.txt");
    let text_partial = format!("{text}.part");
    let revision_prefix = format!("live-{session_id}.transcript.r");
    let names = std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to read transcript artifacts: {error}"))?
        .map(|entry| entry.map_err(|error| format!("Failed to read transcript artifact: {error}")))
        .filter_map(|entry| match entry {
            Ok(entry) => entry.file_name().to_str().map(str::to_owned),
            Err(_) => None,
        })
        .filter(|name| {
            name == &text
                || name == &text_partial
                || (name.starts_with(&revision_prefix)
                    && (name.ends_with(".json") || name.ends_with(".json.part")))
        })
        .collect::<HashSet<_>>();
    Ok(names)
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
    use crate::audio::session::{
        SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode,
    };

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

    fn set_old_modified_time(path: &Path) {
        let old = std::time::SystemTime::now()
            .checked_sub(PARTIAL_RECOVERY_TTL + Duration::from_secs(60))
            .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .unwrap();
    }

    #[test]
    fn transcript_text_prefers_final_then_partial() {
        let mut view = live_view(Some("final"), Some("partial"));

        assert_eq!(transcript_text(&view).as_deref(), Some("final"));
        view.final_text = None;
        assert_eq!(transcript_text(&view).as_deref(), Some("partial"));
    }

    #[test]
    fn damaged_complete_commit_past_partial_ttl_is_preserved_and_warned() {
        let dir = test_dir("damaged-commit-ttl");
        let session = SessionId::new("s-damaged-commit-ttl").unwrap();
        let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
        capture.append_pcm16(&[1, 0]).unwrap();
        capture.finalize().unwrap();
        let journal = dir.join(format!("live-{session}.capture.journal.part"));
        std::fs::write(&journal, b"residual journal").unwrap();
        std::fs::write(dir.join(format!("live-{session}.commit.json")), b"{broken").unwrap();
        set_old_modified_time(&dir.join(format!("live-{session}.wav")));
        set_old_modified_time(&journal);

        let catalog = list_session_catalog_from_dir(&dir).unwrap();

        assert!(catalog.sessions.is_empty());
        assert!(catalog
            .maintenance_warnings
            .iter()
            .any(|warning| warning.contains("damaged")));
        assert!(dir.join(format!("live-{session}.wav")).is_file());
        assert!(journal.is_file());
        assert!(list_recoverable_live_sessions_from_dir(&dir)
            .unwrap()
            .is_empty());
        assert!(recover_live_session_in_dir(&dir, session.to_string()).is_err());
        assert!(delete_recoverable_live_session_in_dir(&dir, session.to_string()).is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn valid_recovered_partial_commit_past_partial_ttl_remains_recoverable() {
        let dir = test_dir("recovered-partial-ttl");
        let session = SessionId::new("s-recovered-partial-ttl").unwrap();
        {
            let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
            capture.append_pcm16(&[1, 0]).unwrap();
        }
        recover_live_session_in_dir(&dir, session.to_string()).unwrap();
        for name in [
            format!("live-{session}.wav"),
            format!("live-{session}.capture.journal.part"),
            format!("live-{session}.capture.partial.json"),
            format!("live-{session}.commit.json"),
        ] {
            set_old_modified_time(&dir.join(name));
        }

        let catalog = list_session_catalog_from_dir(&dir).unwrap();

        assert!(catalog
            .sessions
            .iter()
            .any(|saved| saved.recovery_state.as_deref() == Some("recoverable")));
        assert!(dir.join(format!("live-{session}.wav")).is_file());
        assert!(dir.join(format!("live-{session}.commit.json")).is_file());
        delete_recoverable_live_session_in_dir(&dir, session.to_string()).unwrap();
        assert!(!dir.join(format!("live-{session}.wav")).exists());
        assert!(!dir.join(format!("live-{session}.commit.json")).exists());
        std::fs::remove_dir_all(dir).ok();
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
    fn normal_history_scan_ignores_pre_release_timestamp_pairs() {
        let dir = std::env::temp_dir().join(format!("yap-live-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-200.txt");
        let audio = dir.join("live-200.wav");
        let ignored = dir.join("note.txt");
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::write(&audio, b"RIFF").unwrap();
        std::fs::write(&ignored, "not a live session\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn normal_history_scan_leaves_a_wav_only_pre_release_recording_untouched() {
        let dir = test_dir("ignore-legacy-wav");
        let session = SessionId::new("s-migrate-legacy-source").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let legacy = "live-1720656000000.wav";
        std::fs::rename(dir.join(format!("live-{session}.wav")), dir.join(legacy)).unwrap();
        std::fs::remove_file(dir.join(format!("live-{session}.capture.json"))).unwrap();
        std::fs::remove_file(dir.join(format!("live-{session}.commit.json"))).unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
        assert!(dir.join(legacy).is_file());
        assert!(!dir.join(format!("live-{session}.capture.json")).exists());
        assert!(!dir.join(format!("live-{session}.commit.json")).exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn normal_history_scan_leaves_pre_release_wav_and_txt_untouched() {
        let dir = test_dir("ignore-legacy-pair");
        let session = SessionId::new("s-migrate-legacy-pair-source").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let legacy_wav = "live-1720656000001.wav";
        let legacy_txt = "live-1720656000001.txt";
        std::fs::rename(
            dir.join(format!("live-{session}.wav")),
            dir.join(legacy_wav),
        )
        .unwrap();
        std::fs::remove_file(dir.join(format!("live-{session}.capture.json"))).unwrap();
        std::fs::remove_file(dir.join(format!("live-{session}.commit.json"))).unwrap();
        std::fs::write(dir.join(legacy_txt), "old transcript\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
        assert!(dir.join(legacy_wav).is_file());
        assert_eq!(
            std::fs::read_to_string(dir.join(legacy_txt)).unwrap(),
            "old transcript\n"
        );
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
    fn recovery_patches_a_private_wav_and_publishes_only_partial_metadata() {
        let dir = test_dir("recover-private-wav");
        let session = SessionId::new("s-recover-private-wav").unwrap();
        {
            let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
            recording.append_pcm16(&[1, 0, 2, 0]).unwrap();
        }

        let recoverable = list_recoverable_live_sessions_from_dir(&dir).unwrap();
        assert_eq!(recoverable.len(), 1);
        let saved = recover_live_session_in_dir(&dir, session.to_string()).unwrap();

        assert_eq!(saved.recovery_state.as_deref(), Some("recoverable"));
        assert!(dir.join(format!("live-{session}.wav")).is_file());
        let commit =
            std::fs::read_to_string(dir.join(format!("live-{session}.commit.json"))).unwrap();
        assert!(commit.contains("\"status\":\"partial\""));
        assert!(list_session_files_from_dir(&dir)
            .unwrap()
            .iter()
            .any(|entry| entry.recovery_state.as_deref() == Some("recoverable")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recovery_retry_returns_the_existing_verified_partial_commit() {
        let dir = test_dir("recover-retry");
        let session = SessionId::new("s-recover-retry").unwrap();
        {
            let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
            recording.append_pcm16(&[1, 0, 2, 0]).unwrap();
        }

        let first = recover_live_session_in_dir(&dir, session.to_string()).unwrap();
        let retry = recover_live_session_in_dir(&dir, session.to_string()).unwrap();

        assert_eq!(retry.capture_commit_path, first.capture_commit_path);
        assert_eq!(retry.source_path, first.source_path);
        assert_eq!(retry.recovery_state.as_deref(), Some("recoverable"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn journal_owned_final_wav_without_a_partial_sidecar_remains_recoverable() {
        let dir = test_dir("journal-owned-orphan");
        let session = SessionId::new("s-journal-owned-orphan").unwrap();
        {
            let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
            recording.append_pcm16(&[1, 0]).unwrap();
        }
        recording::recover_partial_wav(&dir, &session).unwrap();

        let recoverable = list_recoverable_live_sessions_from_dir(&dir).unwrap();

        assert_eq!(recoverable.len(), 1);
        assert!(recoverable[0]
            .audio_partial_path
            .as_deref()
            .unwrap()
            .ends_with(".wav"));
        assert!(recoverable[0]
            .journal_partial_path
            .as_deref()
            .unwrap()
            .ends_with(".capture.journal.part"));
        assert!(!dir
            .join(format!("live-{session}.capture.partial.json"))
            .exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn orphan_audio_after_sidecar_failure_is_visible_retryable_and_deletable() {
        let dir = test_dir("orphan-audio-retry");
        let session = SessionId::new("s-orphan-audio-retry").unwrap();
        let mut recording = StreamingRecording::create_with_fault(
            &dir,
            session.clone(),
            CommitFaultPoint::AudioSync,
        )
        .unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let invalid_sidecar = dir.join(format!("live-{session}.capture.partial.json"));
        std::fs::write(&invalid_sidecar, "invalid sidecar").unwrap();

        assert!(recover_live_session_in_dir(&dir, session.to_string()).is_err());
        assert!(dir.join(format!("live-{session}.wav")).is_file());
        let partial = list_recoverable_live_sessions_from_dir(&dir).unwrap();
        assert_eq!(partial.len(), 1);
        assert!(partial[0]
            .audio_partial_path
            .as_deref()
            .unwrap()
            .ends_with(".wav"));

        delete_recoverable_live_session_in_dir(&dir, session.to_string()).unwrap();
        assert!(!dir.join(format!("live-{session}.wav")).exists());
        std::fs::remove_file(&invalid_sidecar).ok();

        let mut retry = StreamingRecording::create_with_fault(
            &dir,
            session.clone(),
            CommitFaultPoint::AudioSync,
        )
        .unwrap();
        retry.append_pcm16(&[1, 0]).unwrap();
        retry.finalize().unwrap();
        assert!(recover_live_session_in_dir(&dir, session.to_string()).is_ok());
        assert!(dir.join(format!("live-{session}.commit.json")).is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn recovery_delete_rejects_unknown_sessions_and_preserves_unrelated_files() {
        let dir = test_dir("recover-delete-boundary");
        let session = SessionId::new("s-recover-delete-boundary").unwrap();
        {
            let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
            recording.append_pcm16(&[1, 0]).unwrap();
        }
        let unrelated = dir.join("not-yap.txt");
        std::fs::write(&unrelated, "keep").unwrap();

        assert!(delete_recoverable_live_session_in_dir(&dir, "../outside".into()).is_err());
        delete_recoverable_live_session_in_dir(&dir, session.to_string()).unwrap();

        assert!(unrelated.is_file());
        assert!(!dir.join(format!("live-{session}.wav.part")).exists());
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
    fn committed_history_exposes_its_hash_validated_commit_path() {
        let dir = test_dir("committed-history-commit-path");
        let session_id = SessionId::new("s-history-commit-path").unwrap();
        let mut recording = StreamingRecording::create(&dir, session_id.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        save_finalized_capture_to_dir(&dir, &live_view(Some("hello"), None), Some(capture))
            .unwrap();

        let saved = list_session_files_from_dir(&dir).unwrap().pop().unwrap();
        let serialized = serde_json::to_value(saved).unwrap();

        assert_eq!(
            serialized["captureCommitPath"],
            serde_json::Value::String(
                dir.join(format!("live-{session_id}.commit.json"))
                    .display()
                    .to_string()
            )
        );
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
    fn transcript_receipt_allows_destination_move_and_revalidates_identity() {
        let dir = test_dir("transcript-receipt-handle-lifetime");
        let session = SessionId::new("s-transcript-receipt-handle-lifetime").unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));

        let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

        receipt.revalidate().unwrap();
        let displaced = transcript.with_extension("displaced");
        std::fs::rename(&transcript, &displaced).unwrap();
        std::fs::write(&transcript, "replacement transcript\n").unwrap();
        assert!(displaced.is_file());
        assert!(transcript.is_file());
        assert!(receipt.revalidate().is_err());
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
    fn highest_corrupt_revision_does_not_fall_back_to_a_valid_lower_revision() {
        let dir = test_dir("highest-corrupt-revision");
        let session = SessionId::new("s-highest-corrupt-revision").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let manifest = recording.finalize().unwrap().committed.unwrap().manifest;
        let transcript = dir.join(format!("live-{session}.txt"));
        let receipt = write_new_text_file(&transcript, "first\n").unwrap();
        write_transcript_revision(
            &dir,
            &session,
            &manifest.capture_sidecar_sha256,
            &receipt,
            "first",
            ResultStatus::Complete,
        )
        .unwrap();
        write_transcript_revision(
            &dir,
            &session,
            &manifest.capture_sidecar_sha256,
            &receipt,
            "second",
            ResultStatus::Complete,
        )
        .unwrap();
        std::fs::write(transcript_revision_path(&dir, &session, 2), "tampered").unwrap();

        assert!(!has_valid_transcript_revision(
            &dir,
            &session,
            &manifest.capture_sidecar_sha256,
        ));
        let saved = list_session_files_from_dir(&dir).unwrap().pop().unwrap();
        assert_eq!(saved.output_path, saved.source_path);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn expired_live_meeting_is_deleted_but_future_and_non_live_origins_survive() {
        let dir = test_dir("meeting-retention");
        let start = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let make = |id: &str, origin: SessionOrigin, expiry: u64| {
            let session = SessionId::new(id).unwrap();
            let metadata = SessionMetadata::new(
                session.clone(),
                SessionMode::Meeting,
                origin,
                TriggerMode::Toggle,
                start,
                None,
                None,
                None,
                Vec::new(),
                Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(expiry)),
            )
            .unwrap();
            let mut recording =
                StreamingRecording::create_with_session_metadata(&dir, metadata).unwrap();
            recording.append_pcm16(&[1, 0]).unwrap();
            recording.finalize().unwrap().committed.unwrap().manifest
        };
        let expired = make("s-expired-meeting", SessionOrigin::LiveCapture, 1_010);
        let future = make("s-future-meeting", SessionOrigin::LiveCapture, 2_000);
        let imported = make("s-imported-meeting", SessionOrigin::ImportedFile, 1_010);
        let now = OffsetDateTime::from_unix_timestamp(1_020).unwrap();

        let sessions = list_session_files_from_dir_at(&dir, now).unwrap();
        assert!(!dir
            .join(format!("live-{}.commit.json", expired.session_id))
            .exists());
        assert!(dir
            .join(format!("live-{}.commit.json", future.session_id))
            .exists());
        assert!(dir
            .join(format!("live-{}.commit.json", imported.session_id))
            .exists());
        assert_eq!(sessions.len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn expired_meeting_with_an_incomplete_transcript_chain_is_retained() {
        let dir = test_dir("meeting-retention-incomplete-transcript");
        let session = SessionId::new("s-expired-incomplete-transcript").unwrap();
        let start = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        let metadata = SessionMetadata::new(
            session.clone(),
            SessionMode::Meeting,
            SessionOrigin::LiveCapture,
            TriggerMode::Toggle,
            start,
            None,
            None,
            None,
            Vec::new(),
            Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_010)),
        )
        .unwrap();
        let mut recording =
            StreamingRecording::create_with_session_metadata(&dir, metadata).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let transcript = dir.join(format!("live-{session}.txt"));
        std::fs::write(&transcript, "unbound transcript\n").unwrap();

        let now = OffsetDateTime::from_unix_timestamp(1_020).unwrap();
        let saved = list_session_files_from_dir_at(&dir, now).unwrap();

        assert_eq!(saved.len(), 1);
        assert!(saved[0]
            .warning
            .as_deref()
            .unwrap()
            .contains("cleanup is pending"));
        assert!(dir.join(format!("live-{session}.wav")).is_file());
        assert!(dir.join(format!("live-{session}.capture.json")).is_file());
        assert!(dir.join(format!("live-{session}.commit.json")).is_file());
        assert!(transcript.is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn manual_saved_session_deletion_removes_bound_artifacts_and_intent() {
        let dir = test_dir("manual-session-deletion");
        let session = SessionId::new("s-manual-delete").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let journal = std::fs::read(recording.journal_path_for_test()).unwrap();
        let capture = recording.finalize().unwrap();
        let journal_name = format!("live-{session}.capture.journal.part");
        std::fs::write(dir.join(&journal_name), journal).unwrap();
        save_finalized_capture_to_dir(&dir, &live_view(Some("delete me"), None), Some(capture))
            .unwrap();
        let polished = dir.join(format!("live-{session}.polished.txt"));
        std::fs::write(&polished, "polished\n").unwrap();

        delete_saved_live_session_in_dir(&dir, session.to_string()).unwrap();

        for name in [
            format!("live-{session}.wav"),
            format!("live-{session}.capture.json"),
            format!("live-{session}.txt"),
            format!("live-{session}.transcript.r1.json"),
            format!("live-{session}.polished.txt"),
            journal_name,
            format!("live-{session}.commit.json"),
            format!("live-{session}.deletion.v1.json"),
        ] {
            assert!(!dir.join(name).exists());
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pending_deletion_resumes_after_audio_was_removed_before_a_crash() {
        let dir = test_dir("resume-deletion-after-audio");
        let session = SessionId::new("s-resume-delete").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        save_finalized_capture_to_dir(&dir, &live_view(Some("resume me"), None), Some(capture))
            .unwrap();
        let capture = recording::scan_recordings(&dir)
            .unwrap()
            .complete
            .pop()
            .unwrap();
        let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
        let intent_name = deletion_intent_name(&session);
        write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();
        recording::remove_regular_artifact_if_hash(
            &dir,
            &intent.artifacts[0].name,
            &intent.artifacts[0].sha256,
        )
        .unwrap();

        assert!(list_session_files_from_dir(&dir).unwrap().is_empty());
        assert!(!dir.join(format!("live-{session}.commit.json")).exists());
        assert!(!dir.join(intent_name).exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn hash_mismatched_replacement_is_preserved_and_keeps_deletion_intent() {
        let dir = test_dir("deletion-replacement");
        let session = SessionId::new("s-replacement-delete").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let capture = recording::scan_recordings(&dir)
            .unwrap()
            .complete
            .pop()
            .unwrap();
        let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
        let intent_name = deletion_intent_name(&session);
        write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();
        let audio = &intent.artifacts[0];
        recording::remove_regular_artifact_if_hash(&dir, &audio.name, &audio.sha256).unwrap();
        std::fs::write(dir.join(&audio.name), b"replacement").unwrap();

        assert!(resume_deletion_intent(&dir, &intent_name).is_err());
        assert_eq!(
            std::fs::read(dir.join(&audio.name)).unwrap(),
            b"replacement"
        );
        assert!(dir.join(intent_name).is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_final_intent_is_quarantined_only_before_deletion_has_started() {
        let dir = test_dir("corrupt-intent-recovery");
        let session = SessionId::new("s-corrupt-intent-recovery").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let intent_name = deletion_intent_name(&session);
        std::fs::write(dir.join(&intent_name), b"{\"truncated\"").unwrap();

        delete_saved_live_session_in_dir(&dir, session.to_string()).unwrap();

        assert!(!dir.join(format!("live-{session}.commit.json")).exists());
        assert!(!dir.join(&intent_name).exists());
        assert!(!std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains("deletion.v1.json.delete-")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reconciliation_collects_only_old_foreign_private_deletion_leftovers() {
        let dir = test_dir("private-deletion-leftovers");
        let stale_staging = dir.join(".live-s-stale-leftover.deletion.v1.999999-0.part");
        let stale_quarantine = dir.join(".live-s-stale-leftover.deletion.v1.json.delete-999999-0");
        let active_staging = dir.join(format!(
            ".live-s-active-leftover.deletion.v1.{}-0.part",
            std::process::id()
        ));
        let unknown = dir.join(".live-s-unknown-leftover.deletion.v1.invalid.part");
        for path in [&stale_staging, &stale_quarantine, &active_staging, &unknown] {
            std::fs::write(path, b"leftover").unwrap();
            set_old_modified_time(path);
        }

        let catalog = list_session_catalog_from_dir(&dir).unwrap();

        assert!(!stale_staging.exists());
        assert!(!stale_quarantine.exists());
        assert!(active_staging.is_file());
        assert!(unknown.is_file());
        assert!(catalog
            .maintenance_warnings
            .iter()
            .any(|warning| warning.contains("Unknown private deletion artifact")));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn corrupt_intent_retries_remove_each_verified_quarantine() {
        let dir = test_dir("corrupt-intent-retry-cleanup");
        let session = SessionId::new("s-corrupt-intent-retry-cleanup").unwrap();
        let mut capture = StreamingRecording::create(&dir, session.clone()).unwrap();
        capture.append_pcm16(&[1, 0]).unwrap();
        capture.finalize().unwrap();
        let committed = recording::scan_recordings(&dir)
            .unwrap()
            .complete
            .pop()
            .unwrap();
        let intent = build_deletion_intent(&dir, &committed, "manual").unwrap();
        let intent_path = dir.join(deletion_intent_name(&session));

        for _ in 0..3 {
            std::fs::write(&intent_path, b"{corrupt").unwrap();
            write_deletion_intent(&intent_path, &intent).unwrap();
            assert!(!std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .contains("deletion.v1.json.delete-")));
            std::fs::remove_file(&intent_path).unwrap();
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deletion_intent_publication_rejects_injected_pre_and_post_publication_replacements() {
        let dir = test_dir("intent-publication-barriers");
        let session = SessionId::new("s-intent-publication-barriers").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let capture = recording::scan_recordings(&dir)
            .unwrap()
            .complete
            .pop()
            .unwrap();
        let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
        let intent_path = dir.join(deletion_intent_name(&session));

        let before = intent_path.clone();
        assert!(write_deletion_intent_with_publication_barrier(
            &intent_path,
            &intent,
            move |after| {
                if !after {
                    std::fs::write(&before, b"competing intent").unwrap();
                }
            }
        )
        .is_err());
        assert_eq!(std::fs::read(&intent_path).unwrap(), b"competing intent");
        std::fs::remove_file(&intent_path).unwrap();

        let after = intent_path.clone();
        assert!(write_deletion_intent_with_publication_barrier(
            &intent_path,
            &intent,
            move |published| {
                if published {
                    std::fs::remove_file(&after).unwrap();
                    std::fs::write(&after, b"replacement intent").unwrap();
                }
            }
        )
        .is_err());
        assert_eq!(std::fs::read(&intent_path).unwrap(), b"replacement intent");
        assert!(dir.join(format!("live-{session}.commit.json")).is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn truncated_final_intent_after_progress_is_retained_as_a_catalog_warning() {
        let dir = test_dir("truncated-intent-after-progress");
        let session = SessionId::new("s-truncated-intent-after-progress").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let capture = recording::scan_recordings(&dir)
            .unwrap()
            .complete
            .pop()
            .unwrap();
        let intent = build_deletion_intent(&dir, &capture, "manual").unwrap();
        let intent_name = deletion_intent_name(&session);
        write_deletion_intent(&dir.join(&intent_name), &intent).unwrap();
        let audio = &intent.artifacts[0];
        recording::remove_regular_artifact_if_hash(&dir, &audio.name, &audio.sha256).unwrap();
        std::fs::write(dir.join(&intent_name), b"{\"truncated\"").unwrap();

        let catalog = list_session_catalog_from_dir(&dir).unwrap();

        assert!(catalog.sessions.is_empty());
        assert!(catalog
            .maintenance_warnings
            .iter()
            .any(|warning| warning.contains("pending")));
        assert!(dir.join(&intent_name).is_file());
        assert!(dir.join(format!("live-{session}.capture.json")).is_file());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn forged_deletion_intent_cannot_name_another_session_or_arbitrary_file() {
        let dir = test_dir("forged-deletion-intent");
        let session = SessionId::new("s-forged-intent").unwrap();
        let arbitrary = dir.join("keep-me.txt");
        std::fs::write(&arbitrary, "keep").unwrap();
        let intent_name = deletion_intent_name(&session);
        let forged = DeletionIntent {
            schema_version: DELETION_INTENT_SCHEMA_VERSION,
            session_id: session,
            reason: "manual".into(),
            commit_file: "live-s-forged-intent.commit.json".into(),
            commit_sha256: "0".repeat(64),
            artifacts: vec![
                DeletionArtifact {
                    name: "live-s-forged-intent.wav".into(),
                    sha256: "0".repeat(64),
                },
                DeletionArtifact {
                    name: "live-s-forged-intent.capture.json".into(),
                    sha256: "0".repeat(64),
                },
                DeletionArtifact {
                    name: "keep-me.txt".into(),
                    sha256: recording::sha256_file(&arbitrary).unwrap(),
                },
            ],
        };
        std::fs::write(
            dir.join(&intent_name),
            format!("{}\n", serde_json::to_string(&forged).unwrap()),
        )
        .unwrap();

        let warnings = reconcile_pending_deletion_intents(&dir);

        assert!(arbitrary.is_file());
        assert!(dir.join(intent_name).is_file());
        assert!(warnings.session_warnings.contains_key("s-forged-intent"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audio_only_committed_session_can_be_deleted() {
        let dir = test_dir("audio-only-delete");
        let session = SessionId::new("s-audio-only-delete").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();

        delete_saved_live_session_in_dir(&dir, session.to_string()).unwrap();

        assert!(!dir.join(format!("live-{session}.wav")).exists());
        assert!(!dir.join(format!("live-{session}.commit.json")).exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn canonical_paths_accept_only_the_hash_valid_committed_audio_and_transcript() {
        let dir = test_dir("canonical-paths");
        let session = SessionId::new("s-canonical-paths").unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        let capture = recording.finalize().unwrap();
        save_finalized_capture_to_dir(
            &dir,
            &live_view(Some("canonical text"), None),
            Some(capture),
        )
        .unwrap();
        let audio = dir.join(format!("live-{session}.wav"));
        let transcript = dir.join(format!("live-{session}.txt"));

        assert_eq!(
            canonical_committed_live_path_from_dir(&audio, &dir, false).unwrap(),
            audio.canonicalize().unwrap()
        );
        assert_eq!(
            canonical_committed_live_path_from_dir(&transcript, &dir, true).unwrap(),
            transcript.canonicalize().unwrap()
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dictation_without_retention_never_creates_a_deletion_intent() {
        let dir = test_dir("dictation-no-retention");
        let session = SessionId::new("s-dictation-no-retention").unwrap();
        let metadata = SessionMetadata::new(
            session.clone(),
            SessionMode::Dictation,
            SessionOrigin::LiveCapture,
            TriggerMode::Toggle,
            std::time::SystemTime::UNIX_EPOCH,
            None,
            None,
            None,
            Vec::new(),
            None,
        )
        .unwrap();
        let mut recording =
            StreamingRecording::create_with_session_metadata(&dir, metadata).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();

        list_session_files_from_dir_at(&dir, OffsetDateTime::now_utc()).unwrap();

        assert!(dir.join(format!("live-{session}.commit.json")).is_file());
        assert!(!dir.join(deletion_intent_name(&session)).exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn normal_history_scan_ignores_timestamp_transcripts_and_leaves_them_untouched() {
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

        assert!(sessions.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("live-205.txt")).unwrap(),
            "hello\n"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn normal_history_scan_ignores_uncommitted_directory_shaped_entries() {
        let dir = std::env::temp_dir().join(format!("yap-live-dir-scan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript_dir = dir.join("live-203.txt");
        let transcript = dir.join("live-204.txt");
        let audio_dir = dir.join("live-204.wav");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(&transcript, "hello\n").unwrap();
        std::fs::create_dir_all(&audio_dir).unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
        assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "hello\n");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saved_live_session_scan_does_not_rewrite_streaming_artifacts() {
        let dir = std::env::temp_dir().join(format!("yap-live-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-201.txt");
        std::fs::write(&transcript, "  THank   you.. \n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
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

        assert!(sessions.is_empty());
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
    fn normal_history_scan_does_not_use_timestamp_filenames_as_history_metadata() {
        let dir = std::env::temp_dir().join(format!("yap-live-timestamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("live-999-1.txt");
        std::fs::write(&transcript, "hello\n").unwrap();

        let sessions = list_session_files_from_dir(&dir).unwrap();

        assert!(sessions.is_empty());
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
