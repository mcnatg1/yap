use std::collections::{HashMap, HashSet};
use std::path::Path;

use time::OffsetDateTime;

use crate::audio::recording;

use super::deletion::{
    delete_committed_session_in_dir_with_publication_barrier_while_owned,
    reconcile_pending_deletion_intents_while_owned,
};
use super::mutation_ownership::session_mutation_ownership_with_queue_observer;
use super::recovery::{
    damaged_commit_warnings, list_recoverable_live_sessions_from_scan, saved_recovered_session,
};
use super::retention::committed_meeting_is_expired;
use super::transcripts::stable_existing_path_string;
use super::{
    committed_at_ms, committed_session_output_path, LiveHistorySourceCatalog, SavedLiveSession,
    SavedLiveSessionCatalog,
};

pub(crate) fn list_history_sources() -> Result<LiveHistorySourceCatalog, String> {
    list_history_sources_from_dir_at_with_queue_observer(
        &recordings_dir(),
        OffsetDateTime::now_utc(),
        || {},
    )
}

#[cfg(test)]
pub(super) fn recordings_dir_from<F>(env: F) -> std::path::PathBuf
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
pub(super) fn list_session_files_from_dir(
    dir: &std::path::Path,
) -> Result<Vec<SavedLiveSession>, String> {
    Ok(list_session_catalog_from_dir_at(dir, OffsetDateTime::now_utc())?.sessions)
}

#[cfg(test)]
pub(super) fn list_session_files_from_dir_at(
    dir: &std::path::Path,
    now: OffsetDateTime,
) -> Result<Vec<SavedLiveSession>, String> {
    Ok(list_session_catalog_from_dir_at(dir, now)?.sessions)
}

#[cfg(test)]
pub(super) fn list_session_catalog_from_dir(
    dir: &std::path::Path,
) -> Result<SavedLiveSessionCatalog, String> {
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
pub(super) fn list_session_catalog_from_dir_at_with_queue_observer<F>(
    dir: &std::path::Path,
    now: OffsetDateTime,
    queued: F,
) -> Result<SavedLiveSessionCatalog, String>
where
    F: FnOnce(),
{
    Ok(list_history_sources_from_dir_at_with_queue_observer(dir, now, queued)?.saved)
}

pub(super) fn list_history_sources_from_dir_at_with_queue_observer<F>(
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
