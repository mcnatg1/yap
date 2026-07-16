//! Owns the native transcript catalog and its durable visibility policy.

mod catalog;
mod visibility;

use std::collections::HashSet;

use crate::jobs::commands::{JobCommandError, RecordingJobs};
use catalog::{
    collect_history_catalog, project_history_catalog, resolve_current_native_identity,
    select_hidden_path_migration,
};
use visibility::HistoryVisibility;

const RECOVERY_WINDOW_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_HISTORY_SESSIONS: usize = 500;
const MAX_HISTORY_PATH_CHARS: usize = 32_768;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
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

impl HistoryCatalogSession {
    fn identity(&self) -> NativeHistoryIdentity {
        NativeHistoryIdentity {
            origin: self.origin,
            session_id: self.session_id.clone(),
            output_path: self.output_path.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NativeHistoryIdentity {
    origin: HistoryOrigin,
    session_id: String,
    output_path: String,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryCatalog {
    maintenance_warnings: Vec<String>,
    sessions: Vec<HistoryCatalogSession>,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HiddenHistoryMigration {
    migrated_output_paths: Vec<String>,
}

pub(crate) struct HistoryCatalogOwner {
    visibility: HistoryVisibility,
}

impl HistoryCatalogOwner {
    pub(crate) fn open_default() -> Self {
        Self {
            visibility: HistoryVisibility::open_default(),
        }
    }

    fn project(&self, mut raw: HistoryCatalog) -> HistoryCatalog {
        match self.visibility.hidden_identities() {
            Ok(hidden) => project_history_catalog(raw, &hidden),
            Err(error) => {
                raw.maintenance_warnings.push(format!(
                    "Hidden history preferences are unavailable: {error}"
                ));
                project_history_catalog(raw, &HashSet::new())
            }
        }
    }

    fn remember_hidden(&self, identities: &[NativeHistoryIdentity]) -> Result<(), String> {
        self.visibility.hide_many(identities)
    }
}

#[tauri::command]
pub(crate) fn history_catalog(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, RecordingJobs>,
    owner: tauri::State<'_, HistoryCatalogOwner>,
) -> Result<HistoryCatalog, JobCommandError> {
    ensure_history_authorized(&window)?;
    Ok(owner.project(load_raw_history_catalog(&jobs)?))
}

#[tauri::command]
pub(crate) fn history_hide_native(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, RecordingJobs>,
    owner: tauri::State<'_, HistoryCatalogOwner>,
    identity: NativeHistoryIdentity,
) -> Result<(), JobCommandError> {
    ensure_history_authorized(&window)?;
    if !valid_native_identity(&identity) {
        return Err(stale_history_identity_error());
    }
    let raw = load_raw_history_catalog(&jobs)?;
    let Some(current) = resolve_current_native_identity(&raw, &identity) else {
        return Err(stale_history_identity_error());
    };
    owner
        .remember_hidden(std::slice::from_ref(&current))
        .map_err(history_visibility_error)
}

#[tauri::command]
pub(crate) fn history_migrate_hidden_paths(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, RecordingJobs>,
    owner: tauri::State<'_, HistoryCatalogOwner>,
    output_paths: Vec<String>,
) -> Result<HiddenHistoryMigration, JobCommandError> {
    ensure_history_authorized(&window)?;
    if output_paths.len() > MAX_HISTORY_SESSIONS {
        return Err(JobCommandError {
            code: "HISTORY_MIGRATION_TOO_LARGE".into(),
            message: format!(
                "Hidden history migration accepts at most {MAX_HISTORY_SESSIONS} paths."
            ),
        });
    }
    if output_paths.is_empty() {
        owner
            .remember_hidden(&[])
            .map_err(history_visibility_error)?;
        return Ok(HiddenHistoryMigration {
            migrated_output_paths: Vec::new(),
        });
    }

    let raw = load_raw_history_catalog(&jobs)?;
    let (identities, migrated_output_paths) = select_hidden_path_migration(&raw, output_paths);
    owner
        .remember_hidden(&identities)
        .map_err(history_visibility_error)?;
    Ok(HiddenHistoryMigration {
        migrated_output_paths,
    })
}

fn ensure_history_authorized(window: &tauri::WebviewWindow) -> Result<(), JobCommandError> {
    crate::authorization::ensure_main(window).map_err(|message| JobCommandError {
        code: "HISTORY_FORBIDDEN".into(),
        message,
    })
}

fn valid_native_identity(identity: &NativeHistoryIdentity) -> bool {
    !identity.session_id.is_empty()
        && identity.session_id.len() <= 128
        && identity
            .session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && !identity.output_path.is_empty()
        && identity.output_path.chars().count() <= MAX_HISTORY_PATH_CHARS
        && !identity.output_path.contains('\0')
}

fn stale_history_identity_error() -> JobCommandError {
    JobCommandError {
        code: "HISTORY_IDENTITY_STALE".into(),
        message: "History identity is no longer current. Refresh history and try again.".into(),
    }
}

fn history_visibility_error(message: String) -> JobCommandError {
    JobCommandError {
        code: "HISTORY_VISIBILITY_ERROR".into(),
        message,
    }
}

fn load_raw_history_catalog(jobs: &RecordingJobs) -> Result<HistoryCatalog, JobCommandError> {
    let live = crate::live::recordings::list_history_sources().map_err(history_error)?;
    let remote = jobs.completed_remote_transcripts()?;
    Ok(collect_history_catalog(
        live.saved,
        live.recoverable,
        remote,
    ))
}

fn history_error(message: String) -> JobCommandError {
    JobCommandError {
        code: "HISTORY_CATALOG_ERROR".into(),
        message,
    }
}
