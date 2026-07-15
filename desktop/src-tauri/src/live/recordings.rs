use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use crate::audio::evidence::ModelRevision;
use crate::audio::recording::{self, PublishedTranscriptReceipt, RecordingFinalizeResult};
use crate::audio::results::{ResultAuthority, ResultStatus, TranscriptResultRevision};
use crate::audio::session::{SessionMode, SessionOrigin};
use crate::live;

const AUDIO_SAVE_FAILED_WARNING: &str = "Live audio could not be saved. Transcript was saved.";
const TRANSCRIPT_DEGRADED_WARNING: &str = "Live transcript may be incomplete. Audio was saved.";
const PARTIAL_RECOVERY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const DELETION_INTENT_SCHEMA_VERSION: u16 = 1;
const MAX_DELETION_ARTIFACTS: usize = 128;
const MAX_MAINTENANCE_WARNINGS: usize = 8;
const MAX_PRIVATE_DELETION_LEFTOVERS: usize = 128;
const PRIVATE_DELETION_LEFTOVER_TTL: Duration = Duration::from_secs(24 * 60 * 60);

static SESSION_MUTATION_OWNERSHIP: OnceLock<SessionMutationOwnership> = OnceLock::new();
static DELETION_CLEANUP_CURSORS: OnceLock<Mutex<HashMap<PathBuf, DeletionCleanupCursors>>> =
    OnceLock::new();

const MAX_DELETION_CLEANUP_CURSOR_DIRS: usize = 64;

#[derive(Default)]
struct DeletionCleanupCursors {
    private_leftovers: Option<String>,
    pending_intents: Option<String>,
}

#[derive(Clone, Copy)]
enum DeletionCleanupCursor {
    PrivateLeftovers,
    PendingIntents,
}

#[derive(Default)]
struct SessionMutationOwnershipState {
    next_ticket: u64,
    serving_ticket: u64,
}

#[derive(Default)]
struct SessionMutationOwnership {
    state: Mutex<SessionMutationOwnershipState>,
    changed: Condvar,
}

struct SessionMutationOwnershipGuard {
    owner: &'static SessionMutationOwnership,
}

impl Drop for SessionMutationOwnershipGuard {
    fn drop(&mut self) {
        let mut state = self
            .owner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.serving_ticket = state.serving_ticket.wrapping_add(1);
        self.owner.changed.notify_all();
    }
}

fn session_mutation_ownership() -> SessionMutationOwnershipGuard {
    session_mutation_ownership_with_queue_observer(|| {})
}

fn session_mutation_ownership_with_queue_observer<F>(queued: F) -> SessionMutationOwnershipGuard
where
    F: FnOnce(),
{
    let owner = SESSION_MUTATION_OWNERSHIP.get_or_init(SessionMutationOwnership::default);
    let mut state = owner
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let ticket = state.next_ticket;
    state.next_ticket = state.next_ticket.wrapping_add(1);
    queued();
    while state.serving_ticket != ticket {
        state = owner
            .changed
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    drop(state);
    SessionMutationOwnershipGuard { owner }
}

fn deletion_cleanup_cursor(dir: &Path, kind: DeletionCleanupCursor) -> Option<String> {
    let cursors = DELETION_CLEANUP_CURSORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let cursors = cursors.get(dir)?;
    match kind {
        DeletionCleanupCursor::PrivateLeftovers => cursors.private_leftovers.clone(),
        DeletionCleanupCursor::PendingIntents => cursors.pending_intents.clone(),
    }
}

fn update_deletion_cleanup_cursor(dir: &Path, kind: DeletionCleanupCursor, cursor: Option<String>) {
    let Some(cursor) = cursor else {
        return;
    };
    let mut cursors = DELETION_CLEANUP_CURSORS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cursors.len() >= MAX_DELETION_CLEANUP_CURSOR_DIRS && !cursors.contains_key(dir) {
        cursors.clear();
    }
    let cursors = cursors.entry(dir.to_path_buf()).or_default();
    match kind {
        DeletionCleanupCursor::PrivateLeftovers => cursors.private_leftovers = Some(cursor),
        DeletionCleanupCursor::PendingIntents => cursors.pending_intents = Some(cursor),
    }
}

struct RotatingDeletionCandidates {
    cursor: Option<String>,
    after: BTreeSet<String>,
    before: BTreeSet<String>,
    overflow: bool,
    limit: usize,
}

impl RotatingDeletionCandidates {
    fn new(cursor: Option<String>, limit: usize) -> Self {
        Self {
            cursor,
            after: BTreeSet::new(),
            before: BTreeSet::new(),
            overflow: false,
            limit,
        }
    }

    fn push(&mut self, name: String) {
        let before_cursor = self.cursor.as_ref().is_some_and(|cursor| name <= *cursor);
        let target = if before_cursor {
            &mut self.before
        } else {
            &mut self.after
        };
        self.overflow |= push_bounded_candidate(target, name, self.limit);
    }

    fn finish(self) -> (BTreeSet<String>, bool, Option<String>) {
        let mut selected = self.after;
        let mut wrapped_last = None;
        let remaining = self.limit.saturating_sub(selected.len());
        for name in self.before.into_iter().take(remaining) {
            wrapped_last = Some(name.clone());
            selected.insert(name);
        }
        let next_cursor = wrapped_last.or_else(|| selected.last().cloned());
        (selected, self.overflow, next_cursor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletionArtifact {
    name: String,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_identity: Option<recording::FileIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletionIntent {
    schema_version: u16,
    session_id: crate::audio::session::SessionId,
    reason: String,
    commit_file: String,
    commit_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    commit_file_identity: Option<recording::FileIdentity>,
    artifacts: Vec<DeletionArtifact>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedLiveSession {
    pub session_id: String,
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

#[derive(Clone, Debug)]
pub(crate) struct LiveHistorySourceCatalog {
    pub(crate) saved: SavedLiveSessionCatalog,
    pub(crate) recoverable: Vec<RecoverableLiveSession>,
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
        session_id: session_id.to_string(),
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
            session_id: capture.session_id.to_string(),
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
        session_id: capture.session_id.to_string(),
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

pub(crate) fn list_history_sources() -> Result<LiveHistorySourceCatalog, String> {
    list_history_sources_from_dir_at_with_queue_observer(
        &recordings_dir(),
        OffsetDateTime::now_utc(),
        || {},
    )
}

pub fn recover_live_session(
    session_id: String,
    expected_artifact_path: String,
) -> Result<SavedLiveSession, String> {
    recover_live_session_in_dir(&recordings_dir(), session_id, expected_artifact_path)
}

pub fn delete_recoverable_live_session(
    session_id: String,
    expected_artifact_path: String,
) -> Result<(), String> {
    delete_recoverable_live_session_in_dir(&recordings_dir(), session_id, expected_artifact_path)
}

pub fn delete_saved_live_session(
    session_id: String,
    expected_output_path: String,
    expected_capture_commit_path: String,
) -> Result<(), String> {
    delete_saved_live_session_in_dir(
        &recordings_dir(),
        session_id,
        expected_output_path,
        expected_capture_commit_path,
    )
}

#[cfg(test)]
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
    if let Some(dir) =
        crate::paths::absolute_env_path(&|key| std::env::var(key).ok(), "YAP_LIVE_RECORDINGS_DIR")
    {
        return dir;
    }
    crate::paths::app_data_dir().join("live-recordings")
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

#[cfg(test)]
fn list_session_catalog_from_dir(dir: &std::path::Path) -> Result<SavedLiveSessionCatalog, String> {
    list_session_catalog_from_dir_at(dir, OffsetDateTime::now_utc())
}

#[cfg(test)]
fn list_session_catalog_from_dir_at(
    dir: &std::path::Path,
    now: OffsetDateTime,
) -> Result<SavedLiveSessionCatalog, String> {
    list_session_catalog_from_dir_at_with_queue_observer(dir, now, || {})
}

#[cfg(test)]
fn list_session_catalog_from_dir_at_with_queue_observer<F>(
    dir: &std::path::Path,
    now: OffsetDateTime,
    queued: F,
) -> Result<SavedLiveSessionCatalog, String>
where
    F: FnOnce(),
{
    Ok(list_history_sources_from_dir_at_with_queue_observer(dir, now, queued)?.saved)
}

fn list_history_sources_from_dir_at_with_queue_observer<F>(
    dir: &std::path::Path,
    now: OffsetDateTime,
    queued: F,
) -> Result<LiveHistorySourceCatalog, String>
where
    F: FnOnce(),
{
    if !dir.exists() {
        return Ok(LiveHistorySourceCatalog {
            saved: SavedLiveSessionCatalog {
                sessions: Vec::new(),
                maintenance_warnings: Vec::new(),
            },
            recoverable: Vec::new(),
        });
    }

    let _ownership = session_mutation_ownership_with_queue_observer(queued);
    list_history_sources_from_dir_at_while_owned(dir, now)
}

fn list_history_sources_from_dir_at_while_owned(
    dir: &std::path::Path,
    now: OffsetDateTime,
) -> Result<LiveHistorySourceCatalog, String> {
    let pending = reconcile_pending_deletion_intents_while_owned(dir);
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
            let audio = dir.join(&committed.manifest.audio_file);
            SavedLiveSession {
                session_id: committed.manifest.session_id.to_string(),
                name,
                source_path: stable_existing_path_string(&audio),
                output_path: stable_existing_path_string(&committed_session_output_path(
                    dir, &committed,
                )),
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
    for recoverable_session in &recoverable {
        let session_id =
            crate::audio::session::SessionId::new(recoverable_session.session_id.clone())?;
        if let Some(saved) = saved_recovered_session(dir, &session_id)? {
            sessions.push(saved);
        }
    }

    sessions.sort_by(|a, b| {
        b.created_at_ms
            .cmp(&a.created_at_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(LiveHistorySourceCatalog {
        saved: SavedLiveSessionCatalog {
            sessions,
            maintenance_warnings,
        },
        recoverable,
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
    delete_committed_session_in_dir_with_publication_barrier_while_owned(
        dir,
        capture,
        "expired-meeting-retention",
        |_| {},
        None,
    )
}

fn committed_session_output_path(dir: &Path, committed: &recording::CommittedCapture) -> PathBuf {
    let transcript = dir.join(format!("live-{}.txt", committed.manifest.session_id));
    if has_valid_transcript_revision(
        dir,
        &committed.manifest.session_id,
        &committed.manifest.capture_sidecar_sha256,
    ) && recording::is_regular_artifact(&transcript)
    {
        transcript
    } else {
        dir.join(&committed.manifest.audio_file)
    }
}

fn admit_expected_artifact_identity(
    actual_path: &Path,
    expected_path: &str,
) -> Result<recording::RegularArtifactIdentity, String> {
    recording::admit_expected_regular_artifact(actual_path, Path::new(expected_path))
}

fn admit_expected_private_artifact_identity(
    actual_path: &Path,
    expected_path: &str,
) -> Result<recording::RegularArtifactIdentity, String> {
    recording::admit_expected_private_regular_artifact(actual_path, Path::new(expected_path))
}

struct ExpectedDeletionArtifacts<'a> {
    output: &'a recording::RegularArtifactIdentity,
    commit: &'a recording::RegularArtifactIdentity,
}

fn delete_saved_live_session_in_dir(
    dir: &Path,
    session_id: String,
    expected_output_path: String,
    expected_capture_commit_path: String,
) -> Result<(), String> {
    let _ownership = session_mutation_ownership();
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    let scan = recording::scan_recordings(dir)?;
    let capture = scan
        .complete
        .iter()
        .find(|capture| capture.manifest.session_id == session_id)
        .ok_or_else(|| "Live recording is not a hash-valid committed Yap session.".to_string())?;
    let expected_output = admit_expected_artifact_identity(
        &committed_session_output_path(dir, capture),
        &expected_output_path,
    )?;
    let expected_commit = admit_expected_artifact_identity(
        &dir.join(format!("live-{session_id}.commit.json")),
        &expected_capture_commit_path,
    )?;
    let expected = ExpectedDeletionArtifacts {
        output: &expected_output,
        commit: &expected_commit,
    };
    delete_committed_session_in_dir_with_publication_barrier_while_owned(
        dir,
        capture,
        "manual",
        |_| {},
        Some(&expected),
    )
}

#[cfg(test)]
fn delete_committed_session_in_dir_with_publication_barrier<F>(
    dir: &Path,
    capture: &recording::CommittedCapture,
    reason: &str,
    publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    let _ownership = session_mutation_ownership();
    delete_committed_session_in_dir_with_publication_barrier_while_owned(
        dir,
        capture,
        reason,
        publication_barrier,
        None,
    )
}

fn delete_committed_session_in_dir_with_publication_barrier_while_owned<F>(
    dir: &Path,
    capture: &recording::CommittedCapture,
    reason: &str,
    publication_barrier: F,
    expected: Option<&ExpectedDeletionArtifacts<'_>>,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    let intent = build_deletion_intent(dir, capture, reason)?;
    let intent_name = deletion_intent_name(&intent.session_id);
    let intent_path = dir.join(&intent_name);
    write_deletion_intent_with_publication_barrier_while_owned(
        &intent_path,
        &intent,
        publication_barrier,
    )?;
    resume_deletion_intent_while_owned_with_expected(dir, &intent_name, expected)
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
    let commit_admission = recording::admit_regular_artifact(&dir.join(&commit_file))?;
    let (commit_text, commit_sha256) = commit_admission.read_and_hash()?;
    let current_manifest: recording::CaptureCommitManifest = serde_json::from_str(&commit_text)
        .map_err(|error| format!("Failed to parse committed recording manifest: {error}"))?;
    if current_manifest != *manifest {
        return Err("committed recording manifest changed before deletion".into());
    }
    let audio = admit_deletion_artifact(dir, &manifest.audio_file)?;
    let sidecar = admit_deletion_artifact(dir, &manifest.capture_sidecar_file)?;
    if audio.sha256 != manifest.audio_sha256 || sidecar.sha256 != manifest.capture_sidecar_sha256 {
        return Err("committed recording artifacts changed before deletion".into());
    }
    let mut artifacts = vec![audio, sidecar];
    let journal = format!("live-{}.capture.journal.part", manifest.session_id);
    if recording::is_regular_artifact(&dir.join(&journal)) {
        let journal_admission = recording::admit_regular_artifact(&dir.join(&journal))?;
        let (journal_text, journal_sha256) = journal_admission.read_and_hash()?;
        let parsed = recording::parse_journal_for_session(&journal_text, &manifest.session_id)?;
        if parsed {
            artifacts.push(DeletionArtifact {
                name: journal,
                sha256: journal_sha256,
                file_identity: Some(journal_admission.file_identity()),
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
            artifacts.push(admit_deletion_artifact(dir, &name)?);
        }
        let polished = format!("live-{}.polished.txt", manifest.session_id);
        if recording::is_regular_artifact(&dir.join(&polished)) {
            artifacts.push(admit_deletion_artifact(dir, &polished)?);
        }
    }
    let intent = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: manifest.session_id.clone(),
        reason: reason.to_string(),
        commit_file,
        commit_sha256,
        commit_file_identity: Some(commit_admission.file_identity()),
        artifacts,
    };
    validate_deletion_intent(&intent)?;
    Ok(intent)
}

fn admit_deletion_artifact(dir: &Path, name: &str) -> Result<DeletionArtifact, String> {
    let admission = recording::admit_regular_artifact(&dir.join(name))?;
    Ok(DeletionArtifact {
        name: name.to_string(),
        sha256: admission.sha256()?,
        file_identity: Some(admission.file_identity()),
    })
}

fn deletion_intent_name(session_id: &crate::audio::session::SessionId) -> String {
    format!("live-{session_id}.deletion.v1.json")
}

#[cfg(test)]
fn write_deletion_intent(path: &Path, intent: &DeletionIntent) -> Result<(), String> {
    write_deletion_intent_with_publication_barrier(path, intent, |_| {})
}

#[cfg(test)]
fn write_deletion_intent_with_publication_barrier<F>(
    path: &Path,
    intent: &DeletionIntent,
    publication_barrier: F,
) -> Result<(), String>
where
    F: FnMut(bool),
{
    let _ownership = session_mutation_ownership();
    write_deletion_intent_with_publication_barrier_while_owned(path, intent, publication_barrier)
}

fn write_deletion_intent_with_publication_barrier_while_owned<F>(
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
    reconcile_intent_evidence_quarantines_while_owned(dir, name)?;
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

#[cfg(test)]
fn reconcile_pending_deletion_intents(dir: &Path) -> ReconciliationWarnings {
    let _ownership = session_mutation_ownership();
    reconcile_pending_deletion_intents_while_owned(dir)
}

fn reconcile_pending_deletion_intents_while_owned(dir: &Path) -> ReconciliationWarnings {
    let mut warnings = ReconciliationWarnings {
        session_warnings: HashMap::new(),
        maintenance_warnings: Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return warnings;
    };
    let mut private_candidates = RotatingDeletionCandidates::new(
        deletion_cleanup_cursor(dir, DeletionCleanupCursor::PrivateLeftovers),
        MAX_PRIVATE_DELETION_LEFTOVERS,
    );
    let mut intent_candidates = RotatingDeletionCandidates::new(
        deletion_cleanup_cursor(dir, DeletionCleanupCursor::PendingIntents),
        MAX_PRIVATE_DELETION_LEFTOVERS,
    );
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if let Some(leftover) = private_deletion_leftover(&name) {
            if let Some((artifact_name, process_id)) = generic_delete_quarantine(&name) {
                if process_id == std::process::id()
                    && deletion_intent_session(artifact_name).is_some()
                {
                    intent_candidates.push(artifact_name.to_string());
                }
            }
            match private_deletion_leftover_is_reconcilable(dir, &name, leftover) {
                Ok(true) => {
                    if let Some((artifact_name, _)) = generic_delete_quarantine(&name) {
                        if deletion_intent_session(artifact_name).is_some() {
                            intent_candidates.push(artifact_name.to_string());
                        }
                    }
                    private_candidates.push(name.clone());
                }
                Ok(false) => {}
                Err(error) => push_maintenance_warning(
                    &mut warnings.maintenance_warnings,
                    format!("Private recording deletion cleanup was retained: {name}: {error}"),
                ),
            }
        } else if looks_like_private_deletion_artifact(&name) {
            push_maintenance_warning(
                &mut warnings.maintenance_warnings,
                format!("Unknown private deletion artifact was retained: {name}"),
            );
        }
        if deletion_intent_session(&name).is_some() {
            intent_candidates.push(name);
        }
    }
    let (private_candidates, private_candidate_overflow, next_private_cursor) =
        private_candidates.finish();
    let (intent_names, intent_candidate_overflow, next_intent_cursor) = intent_candidates.finish();
    update_deletion_cleanup_cursor(
        dir,
        DeletionCleanupCursor::PrivateLeftovers,
        next_private_cursor,
    );
    update_deletion_cleanup_cursor(
        dir,
        DeletionCleanupCursor::PendingIntents,
        next_intent_cursor,
    );
    if private_candidate_overflow {
        push_maintenance_warning(
            &mut warnings.maintenance_warnings,
            "Private recording deletion cleanup scan reached its fixed budget.".into(),
        );
    }
    if intent_candidate_overflow {
        push_maintenance_warning(
            &mut warnings.maintenance_warnings,
            "Recording deletion intent scan reached its fixed budget.".into(),
        );
    }
    for name in &intent_names {
        if let Err(error) = reconcile_intent_evidence_quarantines_while_owned(dir, name) {
            push_maintenance_warning(
                &mut warnings.maintenance_warnings,
                format!("Recording cleanup evidence was retained: {name}: {error}"),
            );
        }
    }
    reconcile_private_deletion_leftovers(
        dir,
        private_candidates.iter().map(String::as_str),
        &mut warnings.maintenance_warnings,
    );
    for name in intent_names {
        if let Err(error) = resume_deletion_intent_while_owned(dir, &name) {
            if let Some(session) = deletion_intent_session(&name) {
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
    Quarantine { process_id: u32 },
}

fn reconcile_private_deletion_leftovers<'a>(
    dir: &Path,
    names: impl IntoIterator<Item = &'a str>,
    warnings: &mut Vec<String>,
) {
    for name in names {
        match physical_entry_exists(dir, name) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => {
                push_maintenance_warning(
                    warnings,
                    format!("Private recording deletion cleanup was retained: {name}: {error}"),
                );
                continue;
            }
        }
        match private_deletion_leftover(name) {
            Some(PrivateDeletionLeftover::Staging { process_id })
            | Some(PrivateDeletionLeftover::Quarantine { process_id }) => {
                let leftover = PrivateDeletionLeftover::Quarantine { process_id };
                match private_deletion_leftover_is_reconcilable(dir, name, leftover) {
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

fn private_deletion_leftover_is_reconcilable(
    dir: &Path,
    name: &str,
    leftover: PrivateDeletionLeftover,
) -> Result<bool, String> {
    let process_id = match leftover {
        PrivateDeletionLeftover::Staging { process_id }
        | PrivateDeletionLeftover::Quarantine { process_id } => process_id,
    };
    if process_id == std::process::id() {
        Ok(false)
    } else {
        private_deletion_leftover_is_old(dir, name)
    }
}

fn intent_evidence_quarantine_is_reconcilable(
    dir: &Path,
    name: &str,
    process_id: u32,
) -> Result<bool, String> {
    if process_id == std::process::id() {
        Ok(true)
    } else {
        private_deletion_leftover_is_old(dir, name)
    }
}

fn private_deletion_leftover(name: &str) -> Option<PrivateDeletionLeftover> {
    if let Some((_, process_id)) = generic_delete_quarantine(name) {
        return Some(PrivateDeletionLeftover::Quarantine { process_id });
    }
    let stem = name.strip_prefix(".live-")?;
    if let Some((session, suffix)) = stem
        .strip_suffix(".part")
        .and_then(|value| value.split_once(".deletion.v1."))
    {
        crate::audio::session::SessionId::new(session.to_string()).ok()?;
        return Some(PrivateDeletionLeftover::Staging {
            process_id: private_process_id(suffix)?,
        });
    }
    None
}

fn generic_delete_quarantine(name: &str) -> Option<(&str, u32)> {
    let (artifact, suffix) = name.strip_prefix('.')?.rsplit_once(".delete-")?;
    session_from_yap_artifact(artifact)?;
    Some((artifact, private_process_id(suffix)?))
}

fn looks_like_private_deletion_artifact(name: &str) -> bool {
    name.starts_with(".live-") && (name.contains(".deletion.v1.") || name.contains(".delete-"))
}

fn private_process_id(value: &str) -> Option<u32> {
    let (process_id, nonce) = value.split_once('-')?;
    if nonce.contains('-') {
        return None;
    }
    nonce.parse::<u64>().ok()?;
    process_id.parse::<u32>().ok()
}

fn session_from_yap_artifact(name: &str) -> Option<crate::audio::session::SessionId> {
    let session = name.strip_prefix("live-")?;
    [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json.part",
        ".capture.partial.json",
        ".commit.json.part",
        ".deletion.v1.json",
        ".commit.json",
        ".capture.json",
        ".polished.txt",
        ".wav",
        ".txt",
    ]
    .into_iter()
    .find_map(|suffix| session.strip_suffix(suffix))
    .and_then(|session| crate::audio::session::SessionId::new(session.to_string()).ok())
    .or_else(|| {
        let (session, revision) = session.rsplit_once(".transcript.r")?;
        revision
            .strip_suffix(".json")?
            .parse::<u64>()
            .ok()
            .filter(|revision| *revision > 0)?;
        crate::audio::session::SessionId::new(session.to_string()).ok()
    })
}

fn deletion_intent_session(name: &str) -> Option<crate::audio::session::SessionId> {
    name.strip_prefix("live-")
        .and_then(|session| session.strip_suffix(".deletion.v1.json"))
        .and_then(|session| crate::audio::session::SessionId::new(session.to_string()).ok())
}

fn push_bounded_candidate(candidates: &mut BTreeSet<String>, name: String, limit: usize) -> bool {
    if candidates.len() < limit {
        candidates.insert(name);
        return false;
    }
    let Some(last) = candidates.last().cloned() else {
        return false;
    };
    if name < last {
        candidates.remove(&last);
        candidates.insert(name);
    }
    true
}

#[cfg(test)]
fn reconcile_intent_evidence_quarantines(dir: &Path, intent_name: &str) -> Result<(), String> {
    let _ownership = session_mutation_ownership();
    reconcile_intent_evidence_quarantines_while_owned(dir, intent_name)
}

fn reconcile_intent_evidence_quarantines_while_owned(
    dir: &Path,
    intent_name: &str,
) -> Result<(), String> {
    let mut newest = None;
    for entry in std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to scan recording deletion evidence: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("Failed to inspect recording deletion evidence: {error}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some((artifact_name, process_id)) = generic_delete_quarantine(&name) else {
            continue;
        };
        if artifact_name != intent_name
            || !intent_evidence_quarantine_is_reconcilable(dir, &name, process_id)?
        {
            continue;
        }
        let file = recording::open_regular_artifact(dir, &name)?;
        let modified = file
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map_err(|error| format!("Failed to inspect recording deletion evidence: {error}"))?;
        if newest.as_ref().is_none_or(
            |(current, current_modified): &(String, std::time::SystemTime)| {
                modified > *current_modified || (modified == *current_modified && name > *current)
            },
        ) {
            newest = Some((name, modified));
        }
    }
    let Some((newest, _)) = newest else {
        return Ok(());
    };
    if !physical_entry_exists(dir, intent_name)? {
        let artifact = recording::verified_regular_artifact(dir, &newest)?;
        recording::restore_verified_quarantined_artifact(&artifact, &dir.join(intent_name))?;
        return reconcile_intent_evidence_quarantines_while_owned(dir, intent_name);
    }
    for entry in std::fs::read_dir(dir)
        .map_err(|error| format!("Failed to scan recording deletion evidence: {error}"))?
    {
        let entry = entry
            .map_err(|error| format!("Failed to inspect recording deletion evidence: {error}"))?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some((artifact_name, process_id)) = generic_delete_quarantine(&name) else {
            continue;
        };
        if artifact_name == intent_name
            && intent_evidence_quarantine_is_reconcilable(dir, &name, process_id)?
        {
            let artifact = recording::verified_regular_artifact(dir, &name)?;
            recording::remove_verified_quarantined_artifact(&artifact)?;
        }
    }
    Ok(())
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

#[cfg(test)]
fn resume_deletion_intent(dir: &Path, intent_name: &str) -> Result<(), String> {
    let _ownership = session_mutation_ownership();
    resume_deletion_intent_while_owned(dir, intent_name)
}

fn resume_deletion_intent_while_owned(dir: &Path, intent_name: &str) -> Result<(), String> {
    resume_deletion_intent_while_owned_with_expected(dir, intent_name, None)
}

fn resume_deletion_intent_while_owned_with_expected(
    dir: &Path,
    intent_name: &str,
    expected: Option<&ExpectedDeletionArtifacts<'_>>,
) -> Result<(), String> {
    let intent_admission = recording::admit_regular_artifact(&dir.join(intent_name))?;
    let (text, intent_sha256) = intent_admission.read_and_hash()?;
    let intent: DeletionIntent = serde_json::from_str(&text)
        .map_err(|error| format!("Failed to parse recording deletion intent: {error}"))?;
    validate_deletion_intent(&intent)?;
    if intent_name != deletion_intent_name(&intent.session_id) {
        return Err("recording deletion intent name does not match its session".into());
    }
    preflight_deletion_intent_artifacts(dir, &intent)?;
    if physical_entry_exists(dir, &intent.commit_file)? {
        if intent.reason != "recoverable" {
            prove_intent_against_current_commit(dir, &intent, OffsetDateTime::now_utc())?;
            if let Some(expected) = expected {
                recording::revalidate_regular_artifact_identity(expected.commit)?;
                intent
                    .artifacts
                    .iter()
                    .find(|artifact| expected.output.matches_artifact_name(&artifact.name))
                    .ok_or_else(|| {
                        "admitted output artifact is absent from the deletion intent".to_string()
                    })?;
                recording::revalidate_regular_artifact_identity(expected.output)?;
            }
        }
        for artifact in &intent.artifacts {
            remove_intent_artifact_if_present(dir, artifact)?;
        }
        remove_intent_commit(dir, &intent)?;
    } else {
        ensure_intent_artifacts_are_absent(dir, &intent)?;
    }
    recording::remove_regular_artifact_if_identity_and_hash(
        dir,
        intent_name,
        &intent_admission,
        &intent_sha256,
    )
}

fn preflight_deletion_intent_artifacts(dir: &Path, intent: &DeletionIntent) -> Result<(), String> {
    if physical_entry_exists(dir, &intent.commit_file)? {
        revalidate_intent_artifact(
            dir,
            &intent.commit_file,
            &intent.commit_sha256,
            intent.commit_file_identity.as_ref(),
        )?;
    }
    for artifact in &intent.artifacts {
        if physical_entry_exists(dir, &artifact.name)? {
            revalidate_intent_artifact(
                dir,
                &artifact.name,
                &artifact.sha256,
                artifact.file_identity.as_ref(),
            )?;
        }
    }
    Ok(())
}

fn revalidate_intent_artifact(
    dir: &Path,
    name: &str,
    sha256: &str,
    identity: Option<&recording::FileIdentity>,
) -> Result<(), String> {
    let identity = identity.ok_or_else(|| {
        "recording deletion intent lacks durable identity evidence; manual reconciliation is required"
            .to_string()
    })?;
    recording::revalidate_regular_artifact_file_identity_and_hash(dir, name, identity, sha256)
}

fn remove_intent_artifact_if_present(
    dir: &Path,
    artifact: &DeletionArtifact,
) -> Result<(), String> {
    if !physical_entry_exists(dir, &artifact.name)? {
        return Ok(());
    }
    let identity = artifact.file_identity.as_ref().ok_or_else(|| {
        "recording deletion intent lacks durable identity evidence; manual reconciliation is required"
            .to_string()
    })?;
    recording::remove_regular_artifact_if_file_identity_and_hash(
        dir,
        &artifact.name,
        identity,
        &artifact.sha256,
    )
}

fn remove_intent_commit(dir: &Path, intent: &DeletionIntent) -> Result<(), String> {
    let identity = intent.commit_file_identity.as_ref().ok_or_else(|| {
        "recording deletion intent lacks durable identity evidence; manual reconciliation is required"
            .to_string()
    })?;
    recording::remove_regular_artifact_if_file_identity_and_hash(
        dir,
        &intent.commit_file,
        identity,
        &intent.commit_sha256,
    )
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
    if intent.reason != "recoverable"
        && prove_intent_against_current_commit(dir, intent, OffsetDateTime::now_utc()).is_err()
    {
        return Ok(false);
    }
    if revalidate_intent_artifact(
        dir,
        &intent.commit_file,
        &intent.commit_sha256,
        intent.commit_file_identity.as_ref(),
    )
    .is_err()
    {
        return Ok(false);
    }
    for artifact in &intent.artifacts {
        if !physical_entry_exists(dir, &artifact.name)?
            || revalidate_intent_artifact(
                dir,
                &artifact.name,
                &artifact.sha256,
                artifact.file_identity.as_ref(),
            )
            .is_err()
        {
            return Ok(false);
        }
    }
    Ok(true)
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
    let recoverable = intent.reason == "recoverable";
    if intent.schema_version != DELETION_INTENT_SCHEMA_VERSION
        || (!recoverable
            && intent.reason != "manual"
            && intent.reason != "expired-meeting-retention")
        || (!recoverable && intent.commit_file != format!("live-{}.commit.json", intent.session_id))
        || (recoverable
            && !is_deletion_artifact_for_session(&intent.commit_file, &intent.session_id))
        || (!recoverable && intent.artifacts.is_empty())
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
    if intent.commit_file_identity.is_none()
        || intent
            .artifacts
            .iter()
            .any(|artifact| artifact.file_identity.is_none())
    {
        return Err(
            "recording deletion intent lacks complete durable identity evidence; manual reconciliation is required"
                .into(),
        );
    }
    if !recoverable
        && (!names.contains(&format!("live-{}.wav", intent.session_id))
            || !names.contains(&format!("live-{}.capture.json", intent.session_id)))
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
        || name == format!("{stem}.wav.part")
        || name == format!("{stem}.capture.json")
        || name == format!("{stem}.capture.json.part")
        || name == format!("{stem}.capture.journal.part")
        || name == format!("{stem}.capture.partial.json")
        || name == format!("{stem}.capture.partial.json.part")
        || name == format!("{stem}.commit.json")
        || name == format!("{stem}.commit.json.part")
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
    if parent != owned_dir || !is_transcript_path(std::path::Path::new(name)) {
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

#[cfg(test)]
fn list_recoverable_live_sessions_from_dir(
    dir: &Path,
) -> Result<Vec<RecoverableLiveSession>, String> {
    let _ownership = session_mutation_ownership();
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
            let cleanup = delete_recoverable_session_artifacts_while_owned(dir, session_id, None);
            if let Err(error) = cleanup {
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

fn damaged_commit_warnings(scan: &recording::RecordingScan, warnings: Vec<String>) -> Vec<String> {
    let mut prioritized = Vec::new();
    for damaged in &scan.damaged {
        push_maintenance_warning(
            &mut prioritized,
            format!(
                "Damaged live recording {} was preserved: {}",
                damaged.session_id, damaged.reason
            ),
        );
    }
    for warning in warnings {
        push_maintenance_warning(&mut prioritized, warning);
    }
    prioritized
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

fn recoverable_session_artifact_path(session: &RecoverableLiveSession) -> Option<&str> {
    session
        .audio_partial_path
        .as_deref()
        .or(session.journal_partial_path.as_deref())
}

fn saved_session_action_artifact_path(session: &SavedLiveSession) -> &str {
    if session.recovery_state.is_some() {
        &session.source_path
    } else {
        &session.output_path
    }
}

fn recover_live_session_in_dir(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
) -> Result<SavedLiveSession, String> {
    recover_live_session_in_dir_with_mutation_barrier(
        dir,
        session_id,
        expected_artifact_path,
        || {},
    )
}

fn recover_live_session_in_dir_with_mutation_barrier<F>(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
    mutation_barrier: F,
) -> Result<SavedLiveSession, String>
where
    F: FnOnce(),
{
    recover_live_session_in_dir_with_queue_observer(
        dir,
        session_id,
        expected_artifact_path,
        || {},
        mutation_barrier,
    )
}

fn recover_live_session_in_dir_with_queue_observer<F, Q>(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
    queued: Q,
    mutation_barrier: F,
) -> Result<SavedLiveSession, String>
where
    F: FnOnce(),
    Q: FnOnce(),
{
    let _ownership = session_mutation_ownership_with_queue_observer(queued);
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    if let Some(saved) = saved_recovered_session(dir, &session_id)? {
        let expected = admit_expected_private_artifact_identity(
            Path::new(saved_session_action_artifact_path(&saved)),
            &expected_artifact_path,
        )?;
        mutation_barrier();
        recording::revalidate_regular_artifact_identity(&expected)?;
        return Ok(saved);
    }
    ensure_recoverable_session(dir, &session_id)?;
    let candidate = recoverable_session_from_dir(dir, &session_id)?;
    let artifact_path = recoverable_session_artifact_path(&candidate).ok_or_else(|| {
        "Recoverable live recording has no authoritative artifact identity.".to_string()
    })?;
    let expected = admit_expected_private_artifact_identity(
        Path::new(artifact_path),
        &expected_artifact_path,
    )?;
    if candidate.expires_at_ms <= unix_millis_now()? {
        return Err("Recoverable live recording has expired.".into());
    }
    if candidate.audio_partial_path.is_none() {
        return Err("Recoverable live recording has no safe partial WAV to repair.".into());
    }
    mutation_barrier();
    let (audio_file, audio_bytes, audio_sha256) =
        recording::recover_partial_wav_with_identity(dir, &session_id, &expected)?;
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

fn delete_recoverable_live_session_in_dir(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
) -> Result<(), String> {
    delete_recoverable_live_session_in_dir_with_mutation_barrier(
        dir,
        session_id,
        expected_artifact_path,
        || {},
    )
}

fn delete_recoverable_live_session_in_dir_with_mutation_barrier<F>(
    dir: &Path,
    session_id: String,
    expected_artifact_path: String,
    mutation_barrier: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    let _ownership = session_mutation_ownership();
    let session_id = crate::audio::session::SessionId::new(session_id)?;
    ensure_recoverable_session(dir, &session_id)?;
    let candidate = recoverable_session_from_dir(dir, &session_id)?;
    let artifact_path = recoverable_session_artifact_path(&candidate).ok_or_else(|| {
        "Recoverable live recording has no authoritative artifact identity.".to_string()
    })?;
    let expected = admit_expected_private_artifact_identity(
        Path::new(artifact_path),
        &expected_artifact_path,
    )?;
    delete_recoverable_session_artifacts_with_barrier_while_owned(
        dir,
        &session_id,
        Some(&expected),
        mutation_barrier,
    )
}

fn delete_recoverable_session_artifacts_while_owned(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
) -> Result<(), String> {
    delete_recoverable_session_artifacts_with_barrier_while_owned(dir, session_id, expected, || {})
}

fn delete_recoverable_session_artifacts_with_barrier_while_owned<F>(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
    mutation_barrier: F,
) -> Result<(), String>
where
    F: FnOnce(),
{
    ensure_recoverable_session(dir, session_id)?;
    let intent = build_recoverable_deletion_intent(dir, session_id, expected)?;
    let intent_name = deletion_intent_name(session_id);
    write_deletion_intent_with_publication_barrier_while_owned(
        &dir.join(&intent_name),
        &intent,
        |_| {},
    )?;
    mutation_barrier();
    resume_deletion_intent_while_owned(dir, &intent_name)
}

fn build_recoverable_deletion_intent(
    dir: &Path,
    session_id: &crate::audio::session::SessionId,
    expected: Option<&recording::RegularArtifactIdentity>,
) -> Result<DeletionIntent, String> {
    let mut names = BTreeSet::new();
    for suffix in [
        ".wav.part",
        ".capture.journal.part",
        ".capture.json.part",
        ".capture.partial.json.part",
        ".commit.json.part",
        ".capture.partial.json",
        ".commit.json",
        ".wav",
        ".txt",
        ".polished.txt",
    ] {
        let name = format!("live-{session_id}{suffix}");
        if regular_artifact_exists(dir, &name) {
            names.insert(name);
        }
    }
    names.extend(transcript_artifact_names(dir, session_id)?);

    let sidecar_name = format!("live-{session_id}.capture.partial.json");
    if names.contains(&sidecar_name)
        && recording::read_regular_artifact(dir, &sidecar_name)
            .is_ok_and(|text| !valid_partial_sidecar(&text, session_id))
    {
        names.remove(&sidecar_name);
    }
    let commit_name = format!("live-{session_id}.commit.json");
    if names.contains(&commit_name) && saved_recovered_session(dir, session_id)?.is_none() {
        names.remove(&commit_name);
    }

    let mut artifacts = names
        .into_iter()
        .map(|name| admit_deletion_artifact(dir, &name))
        .collect::<Result<Vec<_>, _>>()?;
    let anchor_index = if let Some(expected) = expected {
        artifacts
            .iter()
            .position(|artifact| {
                expected.matches_artifact_name(&artifact.name)
                    && artifact.file_identity == Some(expected.file_identity())
            })
            .ok_or_else(|| {
                "admitted recovery artifact is absent from the deletion snapshot".to_string()
            })?
    } else {
        (!artifacts.is_empty())
            .then_some(0)
            .ok_or_else(|| "Recoverable live recording has no deletable artifacts.".to_string())?
    };
    let anchor = artifacts.remove(anchor_index);
    let intent = DeletionIntent {
        schema_version: DELETION_INTENT_SCHEMA_VERSION,
        session_id: session_id.clone(),
        reason: "recoverable".into(),
        commit_file: anchor.name,
        commit_sha256: anchor.sha256,
        commit_file_identity: anchor.file_identity,
        artifacts,
    };
    validate_deletion_intent(&intent)?;
    Ok(intent)
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
        session_id: session_id.to_string(),
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
    is_transcript_path(path)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.starts_with("live-s-"))
}

pub(crate) fn is_transcript_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
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
mod tests;
