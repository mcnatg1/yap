use crate::{
    commands::media_protocol::MediaOwner,
    file_actions::{
        RecordingJobSourceAdmission, RecordingJobSourceError, ValidatedRecordingJobSource,
    },
    jobs::{JobLedger, JobLedgerError, RecordingJobView},
};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

mod catalog;
mod imports;
mod lifecycle;
mod playback;
mod state;

const PENDING_JOB_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_RECORDING_JOBS: usize = 200;
const PHASE5_REMOTE_IMPORT_EXTENSIONS: &[&str] = &["wav"];
static NEXT_JOB_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCommandError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedRemoteTranscriptCatalog {
    pub sessions: Vec<CompletedRemoteTranscript>,
    pub maintenance_warnings: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedRemoteTranscript {
    pub session_id: String,
    pub name: String,
    pub source_path: String,
    pub output_path: String,
    pub created_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl From<JobLedgerError> for JobCommandError {
    fn from(error: JobLedgerError) -> Self {
        Self {
            code: "JOB_LEDGER_ERROR".into(),
            message: error.to_string(),
        }
    }
}

#[doc(hidden)]
pub struct RecordingJobs {
    ledger: JobLedger,
    mutation: Mutex<()>,
    playback: Mutex<HashMap<String, CachedPlayback>>,
    #[cfg(test)]
    projection_failures: Mutex<VecDeque<JobCommandError>>,
    owned_dir: PathBuf,
    remote_jobs_directory: PathBuf,
    registry_path: PathBuf,
    selection_registry_path: PathBuf,
}

struct CachedPlayback {
    source: ValidatedRecordingJobSource,
    playback_path: String,
}

#[tauri::command]
pub(crate) fn recording_jobs_snapshot(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, RecordingJobs>,
    media: tauri::State<'_, MediaOwner>,
) -> Result<Vec<RecordingJobView>, JobCommandError> {
    ensure_main(&window)?;
    jobs.snapshot(&media, now_ms()?)
}

#[tauri::command]
pub(crate) async fn recording_jobs_pick_imports(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Vec<RecordingJobView>, JobCommandError> {
    ensure_main(&window)?;
    #[cfg(feature = "wdio")]
    if let Some(paths) = wdio_picker_override()? {
        return import_native_paths(&app, paths);
    }
    let picker_app = app.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        picker_app
            .dialog()
            .file()
            .set_title("Choose recordings")
            .add_filter("Canonical WAV audio", PHASE5_REMOTE_IMPORT_EXTENSIONS)
            .blocking_pick_files()
    })
    .await
    .map_err(|error| command_error("PICKER_UNAVAILABLE", error.to_string()))?;
    let Some(selected) = selected else {
        return Ok(Vec::new());
    };
    let paths = selected
        .into_iter()
        .map(|path| {
            path.into_path().map_err(|error| {
                command_error(
                    "PICKER_PATH_UNAVAILABLE",
                    format!("The selected recording path is unavailable: {error}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    import_native_paths(&app, paths)
}

#[cfg(feature = "wdio")]
fn wdio_picker_override() -> Result<Option<Vec<PathBuf>>, JobCommandError> {
    let Some(path) = std::env::var_os("YAP_WDIO_PICKER_PATH") else {
        return Ok(None);
    };
    let run_root = std::env::var_os("YAP_WDIO_RUN_ROOT").ok_or_else(|| {
        command_error(
            "WDIO_PICKER_SCOPE_MISSING",
            "The WDIO picker override requires an isolated run root.",
        )
    })?;
    let run_root = PathBuf::from(run_root)
        .canonicalize()
        .map_err(|error| command_error("WDIO_PICKER_SCOPE_INVALID", error.to_string()))?;
    let path = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| command_error("WDIO_PICKER_PATH_INVALID", error.to_string()))?;
    if !path.starts_with(&run_root) {
        return Err(command_error(
            "WDIO_PICKER_PATH_OUTSIDE_RUN",
            "The WDIO picker path is outside the isolated run root.",
        ));
    }
    Ok(Some(vec![path]))
}

#[tauri::command]
pub(crate) fn recording_job_cancel(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    jobs: tauri::State<'_, RecordingJobs>,
    media: tauri::State<'_, MediaOwner>,
    job_id: String,
) -> Result<RecordingJobView, JobCommandError> {
    ensure_main(&window)?;
    mutate_then_notify(
        || jobs.cancel(&media, &job_id, now_ms()?, || {}),
        || emit_jobs_changed(&app),
    )
}

#[tauri::command]
pub(crate) fn recording_job_dismiss(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    jobs: tauri::State<'_, RecordingJobs>,
    media: tauri::State<'_, MediaOwner>,
    job_id: String,
) -> Result<RecordingJobView, JobCommandError> {
    ensure_main(&window)?;
    mutate_then_notify(
        || jobs.dismiss(&media, &job_id, now_ms()?, || {}),
        || emit_jobs_changed(&app),
    )
}

#[tauri::command]
pub(crate) fn recording_job_retry(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    jobs: tauri::State<'_, RecordingJobs>,
    media: tauri::State<'_, MediaOwner>,
    job_id: String,
) -> Result<RecordingJobView, JobCommandError> {
    ensure_main(&window)?;
    mutate_then_notify(
        || jobs.retry(&media, &job_id, now_ms()?, || {}),
        || emit_jobs_changed(&app),
    )
}

fn ensure_main(window: &tauri::WebviewWindow) -> Result<(), JobCommandError> {
    crate::authorization::ensure_main(window)
        .map_err(|message| command_error("UNAUTHORIZED_WINDOW", message))
}

fn now_ms() -> Result<u64, JobCommandError> {
    crate::live::recordings::unix_millis_now()
        .map_err(|message| command_error("CLOCK_UNAVAILABLE", message))
}

fn emit_jobs_changed(app: &tauri::AppHandle) {
    if let Err(error) = app.emit_to(
        crate::authorization::MAIN_WINDOW_LABEL,
        "recording-jobs-changed",
        (),
    ) {
        crate::stt::log_yap(&format!(
            "recording jobs event failed after commit: {error}"
        ));
    }
}

pub(crate) fn import_native_paths(
    app: &tauri::AppHandle,
    paths: Vec<PathBuf>,
) -> Result<Vec<RecordingJobView>, JobCommandError> {
    let jobs = app.state::<RecordingJobs>();
    let media = app.state::<MediaOwner>();
    mutate_then_notify(
        || jobs.create_imports(&media, paths, now_ms()?),
        || emit_jobs_changed(app),
    )
}

pub(crate) fn emit_native_import_error(app: &tauri::AppHandle, error: &JobCommandError) {
    let _ = app.emit_to(
        crate::authorization::MAIN_WINDOW_LABEL,
        "recording-jobs-import-error",
        &error.message,
    );
}

enum RetryKind {
    Accepted,
    Retry,
    Unchanged,
}

fn project_with_admission(
    record: crate::jobs::RecordingJobRecord,
    admission: RecordingJobSourceAdmission,
) -> RecordingJobView {
    let mut view = RecordingJobView::from_record(&record);
    view.source_path = Some(admission.canonical_path.display().to_string());
    view.playback_path = Some(admission.playback_path);
    view
}

fn source_error(error: RecordingJobSourceError) -> JobCommandError {
    match error {
        RecordingJobSourceError::Missing => {
            command_error("SOURCE_MISSING", "Recording source no longer exists.")
        }
        RecordingJobSourceError::Unsafe(message) => command_error("SOURCE_UNSAFE", message),
    }
}

fn mint_job_id(path: &Path, now_ms: u64) -> String {
    let nonce = NEXT_JOB_NONCE.fetch_add(1, Ordering::Relaxed);
    let mut hash = Sha256::new();
    hash.update(path.to_string_lossy().as_bytes());
    hash.update(now_ms.to_le_bytes());
    hash.update(nonce.to_le_bytes());
    format!("job-{}", hex_prefix(&hash.finalize(), 24))
}

fn hex_prefix(bytes: &[u8], digits: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .take(digits)
        .map(|nibble| char::from_digit(u32::from(nibble), 16).expect("hex nibble"))
        .collect()
}

fn command_error(code: impl Into<String>, message: impl Into<String>) -> JobCommandError {
    JobCommandError {
        code: code.into(),
        message: message.into(),
    }
}

fn renewed_expiry(now_ms: u64) -> Result<u64, JobCommandError> {
    now_ms.checked_add(PENDING_JOB_LIFETIME_MS).ok_or_else(|| {
        command_error(
            "JOB_TIME_OUT_OF_RANGE",
            "Recording job expiry is outside the supported time range.",
        )
    })
}

fn log_registry_cleanup_failure(action: &str, error: &str) {
    crate::stt::log_yap(&format!(
        "recording job playback registry {action} failed; snapshot reconciliation will retry: {error}"
    ));
}

fn mutate_then_notify<T, E>(
    mutation: impl FnOnce() -> Result<T, E>,
    notify: impl FnOnce(),
) -> Result<T, E> {
    let result = mutation();
    notify();
    result
}

#[cfg(test)]
mod tests;
