use super::super::deletion::push_maintenance_warning;
use super::super::*;
use super::{
    deletion::delete_recoverable_session_artifacts_while_owned, repair::saved_recovered_session,
};

#[cfg(test)]
pub(in crate::live::recordings) fn list_recoverable_live_sessions_from_dir(
    dir: &Path,
) -> Result<Vec<RecoverableLiveSession>, String> {
    let _ownership = session_mutation_ownership();
    list_recoverable_live_sessions_from_scan(
        dir,
        &recording::scan_recordings(dir)?,
        OffsetDateTime::now_utc(),
    )
}

pub(in crate::live::recordings) fn list_recoverable_live_sessions_from_scan(
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

pub(in crate::live::recordings) fn damaged_commit_warnings(
    scan: &recording::RecordingScan,
    warnings: Vec<String>,
) -> Vec<String> {
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

pub(in crate::live::recordings) fn recoverable_session_from_dir(
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

pub(in crate::live::recordings) fn recoverable_session_artifact_path(
    session: &RecoverableLiveSession,
) -> Option<&str> {
    session
        .audio_partial_path
        .as_deref()
        .or(session.journal_partial_path.as_deref())
}

pub(in crate::live::recordings) fn saved_session_action_artifact_path(
    session: &SavedLiveSession,
) -> &str {
    if session.recovery_state.is_some() {
        &session.source_path
    } else {
        &session.output_path
    }
}

pub(in crate::live::recordings) fn artifact_modified_at_ms(dir: &Path, name: &str) -> Option<u64> {
    let file = recording::open_regular_artifact(dir, name).ok()?;
    system_time_to_unix_millis(file.metadata().ok()?.modified().ok()?)
}

pub(in crate::live::recordings) fn regular_artifact_exists(
    dir: &std::path::Path,
    name: &str,
) -> bool {
    recording::open_regular_artifact(dir, name).is_ok()
}
