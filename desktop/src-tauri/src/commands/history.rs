//! Projects every native transcript source through one read-only history contract.

use std::collections::HashSet;

use crate::{
    jobs::commands::{CompletedRemoteTranscriptCatalog, JobCommandError, RecordingJobs},
    live::recordings::{RecoverableLiveSession, SavedLiveSessionCatalog},
};

const RECOVERY_WINDOW_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_HISTORY_SESSIONS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HistoryOrigin {
    Live,
    Remote,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryCatalogSession {
    capture_commit_path: Option<String>,
    created_at_ms: u64,
    name: String,
    origin: HistoryOrigin,
    output_path: String,
    recovery_state: Option<String>,
    session_id: String,
    source_path: String,
    warning: Option<String>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryCatalog {
    maintenance_warnings: Vec<String>,
    sessions: Vec<HistoryCatalogSession>,
}

#[tauri::command]
pub(crate) fn history_catalog(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, RecordingJobs>,
) -> Result<HistoryCatalog, JobCommandError> {
    crate::authorization::ensure_main(&window).map_err(|message| JobCommandError {
        code: "HISTORY_FORBIDDEN".into(),
        message,
    })?;
    let live = crate::live::recordings::list_history_sources().map_err(history_error)?;
    let remote = jobs.completed_remote_transcripts()?;
    Ok(build_history_catalog(live.saved, live.recoverable, remote))
}

fn history_error(message: String) -> JobCommandError {
    JobCommandError {
        code: "HISTORY_CATALOG_ERROR".into(),
        message,
    }
}

fn build_history_catalog(
    live: SavedLiveSessionCatalog,
    recoverable: Vec<RecoverableLiveSession>,
    remote: CompletedRemoteTranscriptCatalog,
) -> HistoryCatalog {
    let mut sessions = live
        .sessions
        .into_iter()
        .map(|session| HistoryCatalogSession {
            capture_commit_path: session.capture_commit_path,
            created_at_ms: session.created_at_ms,
            name: session.name,
            origin: HistoryOrigin::Live,
            output_path: session.output_path,
            recovery_state: session.recovery_state,
            session_id: session.session_id,
            source_path: session.source_path,
            warning: session.warning,
        })
        .chain(recoverable.into_iter().map(|session| {
            let artifact_path = session
                .audio_partial_path
                .or(session.journal_partial_path)
                .unwrap_or_else(|| session.name.clone());
            HistoryCatalogSession {
                capture_commit_path: None,
                created_at_ms: session.expires_at_ms.saturating_sub(RECOVERY_WINDOW_MS),
                name: session.name,
                origin: HistoryOrigin::Live,
                output_path: artifact_path.clone(),
                recovery_state: Some("recoverable".into()),
                session_id: session.session_id,
                source_path: artifact_path,
                warning: Some(session.reason),
            }
        }))
        .chain(
            remote
                .sessions
                .into_iter()
                .map(|session| HistoryCatalogSession {
                    capture_commit_path: None,
                    created_at_ms: session.created_at_ms,
                    name: session.name,
                    origin: HistoryOrigin::Remote,
                    output_path: session.output_path,
                    recovery_state: None,
                    session_id: session.session_id,
                    source_path: session.source_path,
                    warning: session.warning,
                }),
        )
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .created_at_ms
            .cmp(&left.created_at_ms)
            .then_with(|| left.session_id.cmp(&right.session_id))
            .then_with(|| left.origin.cmp(&right.origin))
    });
    sessions.truncate(MAX_HISTORY_SESSIONS);

    let mut seen_warnings = HashSet::new();
    let maintenance_warnings = live
        .maintenance_warnings
        .into_iter()
        .chain(remote.maintenance_warnings)
        .filter(|warning| seen_warnings.insert(warning.clone()))
        .collect();
    HistoryCatalog {
        maintenance_warnings,
        sessions,
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        jobs::commands::{CompletedRemoteTranscript, CompletedRemoteTranscriptCatalog},
        live::recordings::{RecoverableLiveSession, SavedLiveSession, SavedLiveSessionCatalog},
    };

    use super::{build_history_catalog, HistoryOrigin, MAX_HISTORY_SESSIONS, RECOVERY_WINDOW_MS};

    #[test]
    fn catalog_combines_native_sources_with_explicit_provenance() {
        let catalog = build_history_catalog(
            SavedLiveSessionCatalog {
                sessions: vec![SavedLiveSession {
                    session_id: "live-1".into(),
                    name: "Live".into(),
                    source_path: "live.wav".into(),
                    output_path: "live.txt".into(),
                    created_at_ms: 30,
                    warning: None,
                    capture_commit_path: Some("live.commit.json".into()),
                    recovery_state: None,
                }],
                maintenance_warnings: vec!["shared warning".into()],
            },
            vec![RecoverableLiveSession {
                session_id: "recover-1".into(),
                name: "Recover".into(),
                audio_partial_path: Some("recover.wav.part".into()),
                journal_partial_path: None,
                reason: "Interrupted".into(),
                expires_at_ms: RECOVERY_WINDOW_MS + 20,
            }],
            CompletedRemoteTranscriptCatalog {
                sessions: vec![CompletedRemoteTranscript {
                    session_id: "remote-1".into(),
                    name: "Remote".into(),
                    source_path: "source.wav".into(),
                    output_path: "remote.txt".into(),
                    created_at_ms: 10,
                    warning: None,
                }],
                maintenance_warnings: vec!["shared warning".into(), "remote warning".into()],
            },
        );

        assert_eq!(catalog.sessions.len(), 3);
        assert_eq!(catalog.sessions[0].origin, HistoryOrigin::Live);
        assert_eq!(
            catalog.sessions[1].recovery_state.as_deref(),
            Some("recoverable")
        );
        assert_eq!(catalog.sessions[1].created_at_ms, 20);
        assert_eq!(catalog.sessions[2].origin, HistoryOrigin::Remote);
        assert_eq!(
            catalog.maintenance_warnings,
            ["shared warning", "remote warning"]
        );
    }

    #[test]
    fn catalog_is_bounded_to_the_newest_native_sessions() {
        let remote_sessions = (0..=MAX_HISTORY_SESSIONS)
            .map(|index| CompletedRemoteTranscript {
                session_id: format!("remote-{index}"),
                name: format!("Remote {index}"),
                source_path: format!("source-{index}.wav"),
                output_path: format!("remote-{index}.txt"),
                created_at_ms: index as u64,
                warning: None,
            })
            .collect();
        let catalog = build_history_catalog(
            SavedLiveSessionCatalog {
                sessions: Vec::new(),
                maintenance_warnings: Vec::new(),
            },
            Vec::new(),
            CompletedRemoteTranscriptCatalog {
                sessions: remote_sessions,
                maintenance_warnings: Vec::new(),
            },
        );

        assert_eq!(catalog.sessions.len(), MAX_HISTORY_SESSIONS);
        assert_eq!(
            catalog.sessions[0].created_at_ms,
            MAX_HISTORY_SESSIONS as u64
        );
        assert_eq!(catalog.sessions.last().unwrap().created_at_ms, 1);
    }

    #[test]
    fn catalog_keeps_an_orphaned_recoverable_row_visible_by_name() {
        let catalog = build_history_catalog(
            SavedLiveSessionCatalog {
                sessions: Vec::new(),
                maintenance_warnings: Vec::new(),
            },
            vec![RecoverableLiveSession {
                session_id: "orphan".into(),
                name: "live-orphan".into(),
                audio_partial_path: None,
                journal_partial_path: None,
                reason: "Interrupted".into(),
                expires_at_ms: RECOVERY_WINDOW_MS,
            }],
            CompletedRemoteTranscriptCatalog {
                sessions: Vec::new(),
                maintenance_warnings: Vec::new(),
            },
        );

        assert_eq!(catalog.sessions[0].source_path, "live-orphan");
        assert_eq!(catalog.sessions[0].output_path, "live-orphan");
    }
}
