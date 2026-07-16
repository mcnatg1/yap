use std::collections::HashSet;

use crate::{
    jobs::commands::CompletedRemoteTranscriptCatalog,
    live::recordings::{RecoverableLiveSession, SavedLiveSessionCatalog},
};

use super::{
    HistoryCatalog, HistoryCatalogSession, HistoryOrigin, NativeHistoryIdentity,
    MAX_HISTORY_PATH_CHARS, MAX_HISTORY_SESSIONS, RECOVERY_WINDOW_MS,
};

pub(super) fn resolve_current_native_identity(
    catalog: &HistoryCatalog,
    requested: &NativeHistoryIdentity,
) -> Option<NativeHistoryIdentity> {
    catalog
        .sessions
        .iter()
        .map(HistoryCatalogSession::identity)
        .find(|current| current == requested)
}

pub(super) fn select_hidden_path_migration(
    catalog: &HistoryCatalog,
    output_paths: Vec<String>,
) -> (Vec<NativeHistoryIdentity>, Vec<String>) {
    let mut identities = Vec::new();
    let mut seen_identities = HashSet::new();
    let mut migrated_output_paths = Vec::new();
    let mut seen_requested = HashSet::new();
    for output_path in output_paths {
        if output_path.is_empty() || output_path.chars().count() > MAX_HISTORY_PATH_CHARS {
            continue;
        }
        let requested_identity = history_path_identity(&output_path);
        if !seen_requested.insert(requested_identity.clone()) {
            continue;
        }
        let matching = catalog
            .sessions
            .iter()
            .filter(|session| history_path_identity(&session.output_path) == requested_identity);
        let before = identities.len();
        for identity in matching.map(HistoryCatalogSession::identity) {
            if seen_identities.insert(identity.clone()) {
                identities.push(identity);
            }
        }
        if identities.len() > before {
            migrated_output_paths.push(output_path);
        }
    }
    (identities, migrated_output_paths)
}

#[cfg(test)]
fn build_history_catalog(
    live: SavedLiveSessionCatalog,
    recoverable: Vec<RecoverableLiveSession>,
    remote: CompletedRemoteTranscriptCatalog,
) -> HistoryCatalog {
    project_history_catalog(
        collect_history_catalog(live, recoverable, remote),
        &HashSet::new(),
    )
}

pub(super) fn collect_history_catalog(
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

pub(super) fn project_history_catalog(
    mut catalog: HistoryCatalog,
    hidden: &HashSet<NativeHistoryIdentity>,
) -> HistoryCatalog {
    catalog
        .sessions
        .retain(|session| !hidden.contains(&session.identity()));
    catalog.sessions.truncate(MAX_HISTORY_SESSIONS);
    catalog
}

fn history_path_identity(path: &str) -> String {
    let is_windows = path
        .as_bytes()
        .get(1)
        .is_some_and(|separator| *separator == b':')
        || path.starts_with("\\\\")
        || path.starts_with("//");
    if !is_windows {
        return path.to_owned();
    }

    let mut normalized = path.replace('/', "\\");
    if normalized
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("\\\\?\\UNC\\"))
    {
        normalized = format!("\\\\{}", &normalized[8..]);
    } else if normalized
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("\\\\?\\"))
    {
        normalized = normalized[4..].to_owned();
    }
    let unc = normalized.starts_with("\\\\");
    let root_depth = if unc { 2 } else { 1 };
    let mut resolved = Vec::new();
    for segment in normalized.split('\\').filter(|segment| !segment.is_empty()) {
        match segment {
            "." => {}
            ".." if resolved.len() > root_depth => {
                resolved.pop();
            }
            ".." => {}
            _ => resolved.push(segment),
        }
    }
    format!("{}{}", if unc { "\\\\" } else { "" }, resolved.join("\\")).to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::{
        jobs::commands::{CompletedRemoteTranscript, CompletedRemoteTranscriptCatalog},
        live::recordings::{RecoverableLiveSession, SavedLiveSession, SavedLiveSessionCatalog},
    };

    use super::*;

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
    fn catalog_applies_native_visibility_before_the_history_window() {
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
        let raw = collect_history_catalog(
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
        let hidden = HashSet::from([NativeHistoryIdentity {
            origin: HistoryOrigin::Remote,
            session_id: format!("remote-{MAX_HISTORY_SESSIONS}"),
            output_path: format!("remote-{MAX_HISTORY_SESSIONS}.txt"),
        }]);

        let visible = project_history_catalog(raw, &hidden);

        assert_eq!(visible.sessions.len(), MAX_HISTORY_SESSIONS);
        assert_eq!(
            visible.sessions[0].created_at_ms,
            (MAX_HISTORY_SESSIONS - 1) as u64
        );
        assert_eq!(visible.sessions.last().unwrap().created_at_ms, 0);
    }

    #[test]
    fn native_visibility_requires_the_exact_current_catalog_identity() {
        let raw = collect_history_catalog(
            SavedLiveSessionCatalog {
                sessions: Vec::new(),
                maintenance_warnings: Vec::new(),
            },
            Vec::new(),
            CompletedRemoteTranscriptCatalog {
                sessions: vec![CompletedRemoteTranscript {
                    session_id: "remote-1".into(),
                    name: "Remote".into(),
                    source_path: "source.wav".into(),
                    output_path: "remote.txt".into(),
                    created_at_ms: 1,
                    warning: None,
                }],
                maintenance_warnings: Vec::new(),
            },
        );
        let current = raw.sessions[0].identity();
        assert_eq!(
            resolve_current_native_identity(&raw, &current),
            Some(current.clone())
        );

        let mut wrong_session = current.clone();
        wrong_session.session_id = "remote-2".into();
        assert_eq!(resolve_current_native_identity(&raw, &wrong_session), None);
        let mut wrong_path = current;
        wrong_path.output_path = "other.txt".into();
        assert_eq!(resolve_current_native_identity(&raw, &wrong_path), None);
    }

    #[test]
    fn hidden_path_migration_admits_only_current_native_catalog_paths() {
        let raw = collect_history_catalog(
            SavedLiveSessionCatalog {
                sessions: Vec::new(),
                maintenance_warnings: Vec::new(),
            },
            Vec::new(),
            CompletedRemoteTranscriptCatalog {
                sessions: vec![CompletedRemoteTranscript {
                    session_id: "remote-1".into(),
                    name: "Remote".into(),
                    source_path: r"C:\Yap\source.wav".into(),
                    output_path: r"C:\Yap\remote.txt".into(),
                    created_at_ms: 1,
                    warning: None,
                }],
                maintenance_warnings: Vec::new(),
            },
        );

        let (identities, migrated) = select_hidden_path_migration(
            &raw,
            vec![
                "c:/yap/./remote.txt".into(),
                r"C:\YAP\remote.txt".into(),
                r"C:\Other\external.txt".into(),
            ],
        );

        assert_eq!(identities, vec![raw.sessions[0].identity()]);
        assert_eq!(migrated, ["c:/yap/./remote.txt"]);
    }

    #[test]
    fn hidden_path_migration_preserves_newest_first_client_order() {
        let raw = collect_history_catalog(
            SavedLiveSessionCatalog {
                sessions: Vec::new(),
                maintenance_warnings: Vec::new(),
            },
            Vec::new(),
            CompletedRemoteTranscriptCatalog {
                sessions: vec![
                    CompletedRemoteTranscript {
                        session_id: "newest".into(),
                        name: "Newest".into(),
                        source_path: "newest.wav".into(),
                        output_path: "newest.txt".into(),
                        created_at_ms: 2,
                        warning: None,
                    },
                    CompletedRemoteTranscript {
                        session_id: "older".into(),
                        name: "Older".into(),
                        source_path: "older.wav".into(),
                        output_path: "older.txt".into(),
                        created_at_ms: 1,
                        warning: None,
                    },
                ],
                maintenance_warnings: Vec::new(),
            },
        );

        let (identities, migrated) =
            select_hidden_path_migration(&raw, vec!["newest.txt".into(), "older.txt".into()]);

        assert_eq!(
            identities
                .iter()
                .map(|identity| identity.session_id.as_str())
                .collect::<Vec<_>>(),
            ["newest", "older"]
        );
        assert_eq!(migrated, ["newest.txt", "older.txt"]);
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
