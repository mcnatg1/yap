use super::remote;
use crate::{
    commands::media_protocol::MediaOwner,
    file_actions::{
        RecordingJobSourceAdmission, RecordingJobSourceError, ValidatedRecordingJobSource,
    },
    jobs::{
        JobLedger, JobLedgerError, NewRecordingJob, RecordingJobStatus, RecordingJobView,
        RecordingRoute, SessionMode, SessionOrigin, SourceOwnership,
    },
    server_connector::batch::CreateRecordingJobRequest,
};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::VecDeque;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::{Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

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
pub(crate) fn recording_jobs_completed_transcripts(
    window: tauri::WebviewWindow,
    jobs: tauri::State<'_, RecordingJobs>,
) -> Result<CompletedRemoteTranscriptCatalog, JobCommandError> {
    ensure_main(&window)?;
    jobs.completed_remote_transcripts(&crate::paths::app_data_dir().join("remote-jobs"))
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

impl RecordingJobs {
    pub fn open_default() -> Result<Self, JobCommandError> {
        Ok(Self::from_storage(
            JobLedger::open_default()?,
            crate::live::recordings::recordings_dir(),
            crate::paths::app_data_dir().join("remote-jobs"),
            crate::file_actions::recording_job_playback_registry_path(),
            crate::file_actions::recording_job_selection_registry_path(),
        ))
    }

    #[doc(hidden)]
    pub fn open(
        ledger_path: impl AsRef<Path>,
        owned_dir: impl Into<PathBuf>,
        registry_path: impl Into<PathBuf>,
    ) -> Result<Self, JobCommandError> {
        let registry_path = registry_path.into();
        let selection_registry_path = registry_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("recording-native-selection-registry.json");
        let remote_jobs_directory = registry_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("remote-jobs");
        Ok(Self::from_storage(
            JobLedger::open(ledger_path)?,
            owned_dir.into(),
            remote_jobs_directory,
            registry_path,
            selection_registry_path,
        ))
    }

    fn from_storage(
        ledger: JobLedger,
        owned_dir: PathBuf,
        remote_jobs_directory: PathBuf,
        registry_path: PathBuf,
        selection_registry_path: PathBuf,
    ) -> Self {
        Self {
            ledger,
            mutation: Mutex::new(()),
            playback: Mutex::new(HashMap::new()),
            #[cfg(test)]
            projection_failures: Mutex::new(VecDeque::new()),
            owned_dir,
            remote_jobs_directory,
            registry_path,
            selection_registry_path,
        }
    }

    #[cfg(test)]
    fn from_ledger(ledger: JobLedger, authority_dir: &Path) -> Self {
        let owned_dir = authority_dir.join("owned-live-recordings");
        std::fs::create_dir_all(&owned_dir).expect("prepare test owned directory");
        Self::from_storage(
            ledger,
            owned_dir,
            authority_dir.join("remote-jobs"),
            authority_dir.join("recording-job-playback-registry.json"),
            authority_dir.join("recording-native-selection-registry.json"),
        )
    }

    fn create_imports<P: AsRef<Path>>(
        &self,
        media: &MediaOwner,
        paths: Vec<P>,
        now_ms: u64,
    ) -> Result<Vec<RecordingJobView>, JobCommandError> {
        let _mutation = self.mutation.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        if paths.len() > MAX_RECORDING_JOBS {
            return Err(command_error(
                "JOB_LIMIT_EXCEEDED",
                format!("Yap accepts at most {MAX_RECORDING_JOBS} recording jobs."),
            ));
        }
        if paths.iter().any(|path| {
            path.as_ref()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_none_or(|extension| {
                    !PHASE5_REMOTE_IMPORT_EXTENSIONS
                        .iter()
                        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                })
        }) {
            return Err(command_error(
                "REMOTE_MEDIA_UNSUPPORTED",
                "Private-server transcription currently accepts mono PCM16 16 kHz WAV files only.",
            ));
        }
        let sources = paths
            .iter()
            .map(|path| self.validate_source(path.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut new_sources = HashSet::new();
        for source in &sources {
            if self
                .ledger
                .find_recoverable_imported_job_by_source(&source.canonical_path)?
                .is_none()
            {
                new_sources.insert(source.canonical_path.clone());
            }
        }
        let recoverable_count = self.ledger.list_recoverable_jobs()?.len();
        if recoverable_count.saturating_add(new_sources.len()) > MAX_RECORDING_JOBS {
            return Err(command_error(
                "JOB_LIMIT_EXCEEDED",
                format!("Yap accepts at most {MAX_RECORDING_JOBS} recording jobs."),
            ));
        }

        for source in &sources {
            crate::file_actions::register_native_selected_recording_job_source_at(
                source,
                &self.selection_registry_path,
                &self.owned_dir,
            )
            .map_err(source_error)?;
        }

        let mut records_by_source = HashMap::new();
        let mut new_jobs = Vec::new();
        for source in &sources {
            if records_by_source.contains_key(&source.canonical_path) {
                continue;
            }
            if let Some(existing) = self
                .ledger
                .find_recoverable_imported_job_by_source(&source.canonical_path)?
            {
                records_by_source.insert(source.canonical_path.clone(), existing);
                continue;
            }
            new_jobs.push(NewRecordingJob {
                job_id: mint_job_id(&source.canonical_path, now_ms),
                session_mode: SessionMode::Meeting,
                session_origin: SessionOrigin::ImportedFile,
                source_path: Some(source.canonical_path.clone()),
                source_ownership: SourceOwnership::External,
                output_path: None,
                display_name: source
                    .canonical_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Recording")
                    .to_owned(),
                status: RecordingJobStatus::QueuedServer,
                route: Some(RecordingRoute::ServerBatch),
                attempt_count: 0,
                next_attempt_at_ms: None,
                cancellation_requested: false,
                capture_commit_path: None,
                capture_manifest_sha256: None,
                error_code: None,
                error_message: None,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
                expires_at_ms: now_ms.checked_add(PENDING_JOB_LIFETIME_MS),
            });
        }
        for record in self.ledger.insert_jobs(&new_jobs)? {
            let source_path = record
                .source_path
                .clone()
                .expect("new imported job has a source path");
            records_by_source.insert(source_path, record);
        }

        let mut created = Vec::with_capacity(sources.len());
        let mut projected_by_source: HashMap<PathBuf, RecordingJobView> = HashMap::new();
        for source in sources {
            if let Some(projected) = projected_by_source.get(&source.canonical_path) {
                created.push(projected.clone());
                continue;
            }
            let record = records_by_source
                .get(&source.canonical_path)
                .expect("validated source has a committed job")
                .clone();
            let source_path = source.canonical_path.clone();
            let projected = self.project_committed_or_fail(record, source, media, now_ms)?;
            projected_by_source.insert(source_path, projected.clone());
            created.push(projected);
        }
        Ok(created)
    }

    fn completed_remote_transcripts(
        &self,
        remote_jobs_directory: &Path,
    ) -> Result<CompletedRemoteTranscriptCatalog, JobCommandError> {
        let mut sessions = Vec::new();
        let mut omitted_invalid_result = false;
        for record in self.ledger.list_jobs()?.into_iter().filter(|record| {
            matches!(
                record.status,
                RecordingJobStatus::Complete | RecordingJobStatus::Partial
            ) && record.route == Some(RecordingRoute::ServerBatch)
        }) {
            let verified = (|| {
                let output_path = record.output_path.as_deref().ok_or(())?;
                let source_path = record.source_path.as_deref().ok_or(())?;
                let prepared = self
                    .ledger
                    .get_prepared_remote_job(&record.job_id)
                    .map_err(|_| ())?
                    .ok_or(())?;
                let request =
                    CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json)
                        .map_err(|_| ())?;
                let verified =
                    remote::read_published_remote_transcript(output_path, remote_jobs_directory)
                        .map_err(|_| ())?;
                if verified.result.session_id != request.metadata.session_id.as_str()
                    || verified.result.capture_manifest_sha256 != request.capture_manifest.sha256
                    || prepared.capture_manifest_sha256 != request.capture_manifest.sha256
                    || record.capture_manifest_sha256.as_deref()
                        != Some(request.capture_manifest.sha256.as_str())
                {
                    return Err(());
                }
                Ok(CompletedRemoteTranscript {
                    session_id: verified.result.session_id,
                    name: record.display_name.clone(),
                    source_path: source_path.display().to_string(),
                    output_path: output_path.display().to_string(),
                    created_at_ms: record.updated_at_ms,
                    warning: (record.status == RecordingJobStatus::Partial)
                        .then(|| "Server transcript completed with deferred work.".into()),
                })
            })();
            match verified {
                Ok(session) => sessions.push(session),
                Err(()) => omitted_invalid_result = true,
            }
        }
        sessions.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Ok(CompletedRemoteTranscriptCatalog {
            sessions,
            maintenance_warnings: if omitted_invalid_result {
                vec!["A saved private-server transcript could not be verified and was omitted from history.".into()]
            } else {
                Vec::new()
            },
        })
    }

    fn snapshot(
        &self,
        media: &MediaOwner,
        now_ms: u64,
    ) -> Result<Vec<RecordingJobView>, JobCommandError> {
        let _mutation = self.mutation.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        self.ledger.expire_pending_jobs(now_ms)?;
        let (expired_remote_job_ids, _) = self.ledger.enforce_remote_retention(now_ms)?;
        for job_id in expired_remote_job_ids {
            self.remove_remote_spool_best_effort(&job_id, "retention");
        }
        let mut views = Vec::new();
        let mut recoverable_ids = HashSet::new();
        let mut authorized_paths = Vec::new();
        let mut recoverable_paths = Vec::new();
        for record in self.ledger.list_recoverable_jobs()? {
            recoverable_ids.insert(record.job_id.clone());
            if let Some(source_path) = record.source_path.clone() {
                recoverable_paths.push(source_path);
            }
            match self.project_with_playback(record.clone(), media) {
                Ok(view) => {
                    if view.playback_path.is_some() {
                        if let Some(source_path) = record.source_path.clone() {
                            authorized_paths.push(source_path);
                        }
                    }
                    views.push(view);
                }
                Err(error) if error.code == "SOURCE_MISSING" || error.code == "SOURCE_UNSAFE" => {
                    let failed =
                        self.ledger
                            .fail_source_validation(&record.job_id, &error.code, now_ms)?;
                    views.push(self.project_failed_capability_free(&failed, media));
                }
                Err(error) => return Err(error),
            }
        }
        self.reconcile_playback(&recoverable_ids, media)?;
        if let Err(error) = crate::file_actions::reconcile_recording_job_playback_paths_at(
            &authorized_paths,
            &self.registry_path,
        ) {
            log_registry_cleanup_failure("snapshot reconciliation", &error);
        }
        if let Err(error) = crate::file_actions::reconcile_recording_job_playback_paths_at(
            &recoverable_paths,
            &self.selection_registry_path,
        ) {
            log_registry_cleanup_failure("native selection reconciliation", &error);
        }
        Ok(views)
    }

    fn cancel(
        &self,
        media: &MediaOwner,
        job_id: &str,
        now_ms: u64,
        notify: impl FnOnce(),
    ) -> Result<RecordingJobView, JobCommandError> {
        let mutation = self.mutation.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        let record = self.ledger.request_cancellation(job_id, now_ms)?;
        self.release_playback(job_id, media);
        self.remove_all_job_authority_best_effort(record.source_path.as_deref(), "cancellation");
        self.remove_remote_spool_best_effort(job_id, "cancellation");
        let view = RecordingJobView::from_record(&record);
        drop(mutation);
        notify();
        Ok(view)
    }

    fn retry(
        &self,
        media: &MediaOwner,
        job_id: &str,
        now_ms: u64,
        notify: impl FnOnce(),
    ) -> Result<RecordingJobView, JobCommandError> {
        let mutation = self.mutation.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        let current = self.ledger.get_job(job_id)?.ok_or_else(|| {
            command_error(
                "JOB_NOT_FOUND",
                format!("Recording job {job_id:?} was not found."),
            )
        })?;
        let retry_kind = match current.status {
            RecordingJobStatus::Accepted => RetryKind::Accepted,
            RecordingJobStatus::BlockedSetupRequired
            | RecordingJobStatus::BlockedServerUnavailable
            | RecordingJobStatus::BlockedSignInRequired
            | RecordingJobStatus::Failed => RetryKind::Retry,
            RecordingJobStatus::QueuedServer => RetryKind::Unchanged,
            _ => {
                return Err(command_error(
                    "INVALID_JOB_TRANSITION",
                    format!("Recording job {job_id:?} cannot be retried from its current state."),
                ));
            }
        };
        let removes_prior_remote_spool = matches!(&retry_kind, RetryKind::Retry);
        let source = current.source_path.as_deref().ok_or_else(|| {
            command_error("SOURCE_UNSAFE", "Imported recording has no source path.")
        })?;
        let source = self.validate_source(source)?;

        let (record, changed) = match retry_kind {
            RetryKind::Accepted => (
                self.ledger
                    .accept_to_queued_server(job_id, now_ms, renewed_expiry(now_ms)?)?,
                true,
            ),
            RetryKind::Retry => (
                self.ledger.retry_to_queued_server(
                    job_id,
                    now_ms,
                    Some(renewed_expiry(now_ms)?),
                )?,
                true,
            ),
            RetryKind::Unchanged => (current, false),
        };
        if removes_prior_remote_spool {
            self.remove_remote_spool_best_effort(job_id, "retry");
        }
        let view = self.project_committed_or_fail(record, source, media, now_ms)?;
        drop(mutation);
        if changed {
            notify();
        }
        Ok(view)
    }

    fn dismiss(
        &self,
        media: &MediaOwner,
        job_id: &str,
        now_ms: u64,
        notify: impl FnOnce(),
    ) -> Result<RecordingJobView, JobCommandError> {
        let mutation = self.mutation.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        let record = self.ledger.dismiss_failed(job_id, now_ms)?;
        self.release_playback(job_id, media);
        self.remove_all_job_authority_best_effort(record.source_path.as_deref(), "dismissal");
        self.remove_remote_spool_best_effort(job_id, "dismissal");
        let view = RecordingJobView::from_record(&record);
        drop(mutation);
        notify();
        Ok(view)
    }

    fn validate_source(&self, path: &Path) -> Result<ValidatedRecordingJobSource, JobCommandError> {
        crate::file_actions::validate_recording_job_source_at(path, &self.owned_dir)
            .map_err(source_error)
    }

    fn project_with_playback(
        &self,
        record: crate::jobs::RecordingJobRecord,
        media: &MediaOwner,
    ) -> Result<RecordingJobView, JobCommandError> {
        if record.status == RecordingJobStatus::Failed {
            return Ok(self.project_failed_capability_free(&record, media));
        }
        let Some(source) = record.source_path.as_deref() else {
            return Ok(RecordingJobView::from_record(&record));
        };
        let source = self.validate_source(source)?;
        self.project_validated(record, source, media)
    }

    fn project_validated(
        &self,
        record: crate::jobs::RecordingJobRecord,
        source: ValidatedRecordingJobSource,
        media: &MediaOwner,
    ) -> Result<RecordingJobView, JobCommandError> {
        #[cfg(test)]
        if let Some(error) = self
            .projection_failures
            .lock()
            .expect("projection failure injection lock")
            .pop_front()
        {
            return Err(error);
        }
        let mut playback = self.playback.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording playback state is unavailable.",
            )
        })?;
        if let Some(cached) = playback.get(&record.job_id) {
            if cached.source == source {
                let admission = RecordingJobSourceAdmission {
                    canonical_path: source.canonical_path,
                    playback_path: cached.playback_path.clone(),
                };
                return Ok(project_with_admission(record, admission));
            }
        }
        if let Some(stale) = playback.remove(&record.job_id) {
            media.release(&stale.playback_path);
        }
        let admission = crate::file_actions::authorize_registered_recording_job_source_at(
            &source,
            media,
            &self.selection_registry_path,
            &self.registry_path,
            &self.owned_dir,
        )
        .map_err(source_error)?;
        playback.insert(
            record.job_id.clone(),
            CachedPlayback {
                source,
                playback_path: admission.playback_path.clone(),
            },
        );
        Ok(project_with_admission(record, admission))
    }

    fn project_committed_or_fail(
        &self,
        record: crate::jobs::RecordingJobRecord,
        source: ValidatedRecordingJobSource,
        media: &MediaOwner,
        now_ms: u64,
    ) -> Result<RecordingJobView, JobCommandError> {
        if record.status == RecordingJobStatus::Failed {
            return Ok(self.project_failed_capability_free(&record, media));
        }
        match self.project_validated(record.clone(), source, media) {
            Ok(view) => Ok(view),
            Err(error) => {
                let failed =
                    self.ledger
                        .fail_source_validation(&record.job_id, &error.code, now_ms)?;
                Ok(self.project_failed_capability_free(&failed, media))
            }
        }
    }

    fn project_failed_capability_free(
        &self,
        record: &crate::jobs::RecordingJobRecord,
        media: &MediaOwner,
    ) -> RecordingJobView {
        debug_assert_eq!(record.status, RecordingJobStatus::Failed);
        self.release_playback(&record.job_id, media);
        self.remove_active_job_authority_best_effort(
            record.source_path.as_deref(),
            "failed projection",
        );
        let mut view = RecordingJobView::from_record(record);
        view.source_path = None;
        view.playback_path = None;
        view
    }

    #[cfg(test)]
    fn inject_projection_failures_for_test(&self, failures: Vec<JobCommandError>) {
        self.projection_failures
            .lock()
            .expect("projection failure injection lock")
            .extend(failures);
    }

    fn release_playback(&self, job_id: &str, media: &MediaOwner) {
        let removed = self
            .playback
            .lock()
            .ok()
            .and_then(|mut playback| playback.remove(job_id));
        if let Some(removed) = removed {
            media.release(&removed.playback_path);
        }
    }

    fn remove_active_job_authority_best_effort(&self, path: Option<&Path>, action: &str) {
        let Some(path) = path else {
            return;
        };
        if let Err(error) =
            crate::file_actions::remove_recording_job_playback_path_at(path, &self.registry_path)
        {
            log_registry_cleanup_failure(action, &error);
        }
    }

    fn remove_all_job_authority_best_effort(&self, path: Option<&Path>, action: &str) {
        self.remove_active_job_authority_best_effort(path, action);
        let Some(path) = path else {
            return;
        };
        if let Err(error) = crate::file_actions::remove_recording_job_playback_path_at(
            path,
            &self.selection_registry_path,
        ) {
            log_registry_cleanup_failure(&format!("{action} native selection"), &error);
        }
    }

    fn remove_remote_spool_best_effort(&self, job_id: &str, action: &str) {
        if let Err(error) = remote::reset_unattached_spool(job_id, &self.remote_jobs_directory) {
            crate::stt::log_yap(&format!(
                "owned remote recording cleanup after {action} remains pending: {error}"
            ));
        }
    }

    fn reconcile_playback(
        &self,
        recoverable_ids: &HashSet<String>,
        media: &MediaOwner,
    ) -> Result<(), JobCommandError> {
        let mut playback = self.playback.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording playback state is unavailable.",
            )
        })?;
        let stale_ids = playback
            .keys()
            .filter(|job_id| !recoverable_ids.contains(*job_id))
            .cloned()
            .collect::<Vec<_>>();
        for job_id in stale_ids {
            if let Some(stale) = playback.remove(&job_id) {
                media.release(&stale.playback_path);
            }
        }
        Ok(())
    }
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
mod tests {
    use super::*;
    use crate::{commands::media_protocol::MediaOwner, jobs::JobLedger};
    use std::{
        cell::{Cell, RefCell},
        fs,
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, UNIX_EPOCH},
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn mutation_adapter_notifies_even_when_the_operation_returns_an_error() {
        let notified = Cell::new(false);

        let result = mutate_then_notify(
            || Err::<(), _>(command_error("INJECTED_FAILURE", "injected")),
            || notified.set(true),
        );

        assert_eq!(result.unwrap_err().code, "INJECTED_FAILURE");
        assert!(notified.get());
    }

    #[test]
    fn completed_remote_catalog_revalidates_the_immutable_result_before_history_projection() {
        let dir = temp_dir("completed-remote-catalog");
        let database = dir.join("jobs.sqlite3");
        let source_path = dir.join("meeting.wav");
        let remote_jobs = dir.join("remote-jobs");
        write_pcm_wav(&source_path, &vec![0_u8; 320]);
        let mut source = fs::File::open(&source_path).unwrap();
        let owner = crate::audio::session::OwnerNamespace::local("i-catalog-test").unwrap();
        let prepared = remote::prepare_imported_pcm_wav(
            "job-completed-catalog",
            "meeting.wav",
            &mut source,
            &remote_jobs,
            &owner,
            UNIX_EPOCH + Duration::from_secs(1_720_000_000),
        )
        .unwrap();
        let request = prepared.request.clone();
        let durable = prepared.into_ledger_state().unwrap();
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&NewRecordingJob {
                job_id: "job-completed-catalog".into(),
                session_mode: SessionMode::Meeting,
                session_origin: SessionOrigin::ImportedFile,
                source_path: Some(source_path.clone()),
                source_ownership: SourceOwnership::External,
                output_path: None,
                display_name: "meeting.wav".into(),
                status: RecordingJobStatus::Preprocessing,
                route: Some(RecordingRoute::ServerBatch),
                attempt_count: 0,
                next_attempt_at_ms: None,
                cancellation_requested: false,
                capture_commit_path: None,
                capture_manifest_sha256: None,
                error_code: None,
                error_message: None,
                created_at_ms: 1_720_000_000_000,
                updated_at_ms: 1_720_000_000_000,
                expires_at_ms: Some(1_720_604_800_000),
            })
            .unwrap();
        ledger
            .attach_prepared_remote_job("job-completed-catalog", &durable, 1_720_000_000_100)
            .unwrap();
        let server_job_id = "job-0123456789abcdef0123456789abcdef";
        ledger
            .begin_remote_create_attempt(
                "job-completed-catalog",
                "http://127.0.0.1:18765",
                1_720_000_000_200,
            )
            .unwrap();
        ledger
            .record_server_job_id(
                "job-completed-catalog",
                server_job_id,
                "http://127.0.0.1:18765",
                1_720_000_000_200,
            )
            .unwrap();
        for chunk in &request.chunks {
            ledger
                .acknowledge_remote_chunk(
                    "job-completed-catalog",
                    &chunk.replay_key.track_id,
                    chunk.replay_key.sequence_start,
                    chunk.replay_key.sequence_end,
                    &chunk.content_identity.sha256,
                    1_720_000_000_300,
                )
                .unwrap();
        }
        ledger
            .mark_remote_job_committed("job-completed-catalog", 1_720_000_000_400)
            .unwrap();
        ledger
            .begin_remote_result_saving("job-completed-catalog", 1_720_000_000_500)
            .unwrap();
        let result = crate::server_connector::batch::TranscriptResultRevision {
            session_id: request.metadata.session_id.to_string(),
            revision: 1,
            authority: "server_authoritative".into(),
            created_at_utc: "2026-07-14T21:00:02Z".into(),
            capture_manifest_sha256: request.capture_manifest.sha256.clone(),
            previous_result_sha256: None,
            status: "complete".into(),
            language: Some(crate::server_connector::batch::LanguageDecision {
                language_bcp47: "en-US".into(),
                confidence: Some(0.98),
            }),
            transcript: "Catalog result.".into(),
            aligned_words: Vec::new(),
            model_provenance: vec![crate::server_connector::batch::ModelRevision {
                model_id: "CohereLabs/cohere-transcribe-03-2026".into(),
                revision: "b1eacc2686a3d08ceaae5f24a88b1d519620bc09".into(),
                calibration_revision: "asr-not-applicable".into(),
            }],
        };
        let output =
            remote::publish_remote_result("job-completed-catalog", &remote_jobs, &result).unwrap();
        ledger
            .complete_remote_result(
                "job-completed-catalog",
                &output,
                1_722_592_000_000,
                1_720_000_000_600,
            )
            .unwrap();
        let jobs = RecordingJobs::from_ledger(ledger, &dir);

        let catalog = jobs.completed_remote_transcripts(&remote_jobs).unwrap();
        assert_eq!(catalog.sessions.len(), 1);
        assert_eq!(
            catalog.sessions[0].output_path,
            output.display().to_string()
        );
        assert!(catalog.maintenance_warnings.is_empty());

        fs::write(&output, "tampered\n").unwrap();
        let rejected = jobs.completed_remote_transcripts(&remote_jobs).unwrap();
        assert!(rejected.sessions.is_empty());
        assert_eq!(rejected.maintenance_warnings.len(), 1);

        assert!(jobs
            .snapshot(&MediaOwner::new(), 1_722_592_000_000)
            .unwrap()
            .is_empty());
        assert!(!remote_jobs.join("job-completed-catalog").exists());
        assert!(
            source_path.is_file(),
            "external source must never be deleted"
        );

        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_imports_validates_and_native_allowlists_a_canonical_recording() {
        let dir = temp_dir("create-import");
        let source = dir.join("meeting.wav");
        fs::write(&source, b"RIFF-command-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        let created = jobs
            .create_imports(&media, vec![source.display().to_string()], 1_000)
            .unwrap();

        assert_eq!(created.len(), 1);
        assert_eq!(
            created[0].source_path.as_deref(),
            source.canonicalize().unwrap().to_str()
        );
        assert!(created[0]
            .playback_path
            .as_deref()
            .is_some_and(|path| path.starts_with("http://127.0.0.1:")));
        assert_eq!(created[0].id, jobs.snapshot(&media, 1_001).unwrap()[0].id);
        assert!(fs::read_to_string(&jobs.registry_path)
            .unwrap()
            .contains("meeting.wav"));
        assert!(!dir.join("recording-playback-registry.json").exists());

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_imports_rejects_media_that_phase5_cannot_prepare() {
        let dir = temp_dir("create-unsupported-remote-media");
        let source = dir.join("meeting.mp3");
        fs::write(&source, b"not admitted before remote preparation").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        let error = jobs
            .create_imports(&media, vec![source.display().to_string()], 1_000)
            .unwrap_err();

        assert_eq!(error.code, "REMOTE_MEDIA_UNSUPPORTED");
        assert!(error.message.contains("mono PCM16 16 kHz WAV"));
        assert!(jobs.snapshot(&media, 1_001).unwrap().is_empty());

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn authority_failed_create_stays_capability_free_until_explicit_retry() {
        let dir = temp_dir("create-admission-failure");
        let database = dir.join("jobs.sqlite3");
        let general_registry = dir.join("recording-playback-registry.json");
        let source = dir.join("meeting.wav");
        let source_bytes = b"RIFF-command-fixture";
        fs::write(&source, source_bytes).unwrap();
        let canonical_source = source.canonicalize().unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        jobs.inject_projection_failures_for_test(vec![command_error(
            "PLAYBACK_AUTHORITY_FAILED",
            "injected admission failure",
        )]);
        let authority_denied_before_event_snapshot = Cell::new(false);
        let authority_denied_after_event_snapshot = Cell::new(false);
        let event_snapshot = RefCell::new(None);

        let created = mutate_then_notify(
            || jobs.create_imports(&media, vec![source.display().to_string()], 1_500),
            || {
                authority_denied_before_event_snapshot.set(open_and_reveal_are_denied(
                    &jobs,
                    &source,
                    &general_registry,
                ));
                let snapshot = jobs.snapshot(&media, 1_501).unwrap();
                authority_denied_after_event_snapshot.set(open_and_reveal_are_denied(
                    &jobs,
                    &source,
                    &general_registry,
                ));
                *event_snapshot.borrow_mut() = Some(snapshot);
            },
        )
        .unwrap();
        let event_snapshot = event_snapshot.into_inner().unwrap();
        let duplicate = jobs
            .create_imports(&media, vec![source.display().to_string()], 1_502)
            .unwrap();
        let duplicate_authority_denied =
            open_and_reveal_are_denied(&jobs, &source, &general_registry);
        let committed = jobs.ledger.get_job(&created[0].id).unwrap().unwrap();

        assert_eq!(
            committed.error_code.as_deref(),
            Some("PLAYBACK_AUTHORITY_FAILED")
        );
        assert_eq!(
            committed.source_path.as_deref(),
            Some(canonical_source.as_path())
        );
        assert_eq!(duplicate[0].id, created[0].id);
        assert_eq!(fs::read(&source).unwrap(), source_bytes);

        drop(media);
        drop(jobs);

        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let restart_snapshot = jobs.snapshot(&media, 1_503).unwrap();
        let restart_authority_denied =
            open_and_reveal_are_denied(&jobs, &source, &general_registry);
        let restarted = jobs.ledger.get_job(&created[0].id).unwrap().unwrap();
        assert_eq!(
            restarted.source_path.as_deref(),
            Some(canonical_source.as_path())
        );
        assert_eq!(fs::read(&source).unwrap(), source_bytes);

        let observations = [
            ("immediate response", &created[0]),
            ("event snapshot", &event_snapshot[0]),
            ("duplicate create", &duplicate[0]),
            ("restart snapshot", &restart_snapshot[0]),
        ];
        let authority_denials = [
            (
                "before event snapshot",
                authority_denied_before_event_snapshot.get(),
            ),
            (
                "after event snapshot",
                authority_denied_after_event_snapshot.get(),
            ),
            ("after duplicate create", duplicate_authority_denied),
            ("after restart snapshot", restart_authority_denied),
        ];
        assert!(
            authority_denials.iter().all(|(_, denied)| *denied),
            "open/reveal authorization must remain denied: {authority_denials:#?}"
        );
        assert!(
            observations
                .iter()
                .all(|(_, view)| capability_free_failed(view)),
            "every durable failed projection must be capability-free: {observations:#?}"
        );

        let retried = jobs.retry(&media, &created[0].id, 1_504, || {}).unwrap();
        assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
        assert_eq!(retried.source_path.as_deref(), canonical_source.to_str());
        assert!(retried.playback_path.is_some());
        assert_eq!(
            crate::file_actions::openable_app_path_from_registries(
                source.display().to_string(),
                &general_registry,
                &jobs.registry_path,
                &jobs.owned_dir,
            )
            .unwrap(),
            canonical_source
        );
        assert_eq!(fs::read(&source).unwrap(), source_bytes);

        drop(media);
        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pre_native_picker_ledger_and_registry_cannot_reauthorize_on_restart() {
        let dir = temp_dir("pre-native-picker-restart");
        let database = dir.join("jobs.sqlite3");
        let source = dir.join("legacy-renderer-path.wav");
        fs::write(&source, b"RIFF-pre-native-picker-fixture").unwrap();
        let canonical_source = source.canonicalize().unwrap();
        let job_id;
        let active_registry;
        let selection_registry;
        {
            let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
            let media = MediaOwner::new();
            job_id = jobs
                .create_imports(&media, vec![source.clone()], 1_600)
                .unwrap()[0]
                .id
                .clone();
            active_registry = jobs.registry_path.clone();
            selection_registry = jobs.selection_registry_path.clone();
        }
        fs::remove_file(&selection_registry).unwrap();
        fs::write(
            &active_registry,
            format!(
                r#"{{"version":1,"paths":[{}]}}"#,
                serde_json::to_string(&canonical_source.display().to_string()).unwrap()
            ),
        )
        .unwrap();

        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let snapshot = jobs.snapshot(&media, 1_601).unwrap();

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, job_id);
        assert!(capability_free_failed(&snapshot[0]));
        assert_eq!(media.active_admission_count_for_test(), 0);
        assert!(crate::file_actions::openable_app_path_from_registries(
            source.display().to_string(),
            &dir.join("recording-playback-registry.json"),
            &jobs.registry_path,
            &jobs.owned_dir,
        )
        .is_err());
        drop(media);
        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn multi_create_commits_every_row_before_returning_injected_projection_outcomes() {
        let dir = temp_dir("multi-create-admission-failure");
        let failed_source = dir.join("failed.wav");
        let queued_source = dir.join("queued.wav");
        fs::write(&failed_source, b"RIFF-failed-fixture").unwrap();
        fs::write(&queued_source, b"RIFF-queued-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        jobs.inject_projection_failures_for_test(vec![command_error(
            "PLAYBACK_AUTHORITY_FAILED",
            "injected first-row admission failure",
        )]);

        let created = mutate_then_notify(
            || {
                jobs.create_imports(
                    &media,
                    vec![
                        failed_source.display().to_string(),
                        queued_source.display().to_string(),
                    ],
                    1_700,
                )
            },
            || {
                let committed = jobs.ledger.list_recoverable_jobs().unwrap();
                assert_eq!(committed.len(), 2);
                assert_eq!(
                    committed
                        .iter()
                        .filter(|job| job.status == RecordingJobStatus::Failed)
                        .count(),
                    1
                );
                assert_eq!(
                    committed
                        .iter()
                        .filter(|job| job.status == RecordingJobStatus::QueuedServer)
                        .count(),
                    1
                );
            },
        )
        .unwrap();

        assert_eq!(created.len(), 2);
        assert_eq!(created[0].status, RecordingJobStatus::Failed);
        assert_eq!(created[0].playback_path, None);
        assert_eq!(created[1].status, RecordingJobStatus::QueuedServer);
        assert!(created[1].playback_path.is_some());

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn duplicate_file_import_returns_the_existing_rust_minted_job() {
        let dir = temp_dir("duplicate-import");
        let source = dir.join("same.wav");
        fs::write(&source, b"RIFF-duplicate-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let path = source.display().to_string();

        let first = jobs
            .create_imports(&media, vec![path.clone()], 2_000)
            .unwrap();
        let duplicate = jobs.create_imports(&media, vec![path], 2_001).unwrap();

        assert_eq!(duplicate[0].id, first[0].id);
        assert_eq!(duplicate[0].playback_path, first[0].playback_path);
        assert_eq!(media.active_admission_count_for_test(), 1);
        assert_eq!(jobs.snapshot(&media, 2_002).unwrap().len(), 1);
        assert_eq!(media.active_admission_count_for_test(), 1);

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unrelated_mutations_preserve_playback_but_source_replacement_rotates_it() {
        let dir = temp_dir("stable-playback");
        let selected = dir.join("selected.wav");
        let unrelated = dir.join("unrelated.wav");
        let original = dir.join("selected-original.wav");
        fs::write(&selected, b"RIFF-selected-original").unwrap();
        fs::write(&unrelated, b"RIFF-unrelated").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        let selected_job = jobs
            .create_imports(&media, vec![selected.display().to_string()], 2_100)
            .unwrap()[0]
            .clone();
        jobs.create_imports(&media, vec![unrelated.display().to_string()], 2_101)
            .unwrap();
        let after_unrelated = jobs.snapshot(&media, 2_102).unwrap();
        let selected_after_unrelated = after_unrelated
            .iter()
            .find(|job| job.id == selected_job.id)
            .unwrap();
        assert_eq!(
            selected_after_unrelated.playback_path,
            selected_job.playback_path
        );
        assert_eq!(media.active_admission_count_for_test(), 2);

        fs::rename(&selected, &original).unwrap();
        fs::write(&selected, b"RIFF-selected-replacement").unwrap();
        let after_replacement = jobs.snapshot(&media, 2_103).unwrap();
        let selected_after_replacement = after_replacement
            .iter()
            .find(|job| job.id == selected_job.id)
            .unwrap();
        assert_ne!(
            selected_after_replacement.playback_path,
            selected_job.playback_path
        );
        assert_eq!(media.active_admission_count_for_test(), 2);

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_imports_is_all_or_nothing_for_invalid_paths_and_snapshot_is_stably_ordered() {
        let dir = temp_dir("validation-ordering");
        let later = dir.join("later.wav");
        let earlier = dir.join("earlier.wav");
        let missing = dir.join("missing.wav");
        fs::write(&later, b"RIFF-later-fixture").unwrap();
        fs::write(&earlier, b"RIFF-earlier-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        let invalid = jobs.create_imports(
            &media,
            vec![later.display().to_string(), missing.display().to_string()],
            2_500,
        );
        assert_eq!(invalid.unwrap_err().code, "SOURCE_MISSING");
        assert!(jobs.ledger.list_jobs().unwrap().is_empty());
        assert!(!jobs.registry_path.exists());
        assert_eq!(media.active_admission_count_for_test(), 0);

        let later_id = jobs
            .create_imports(&media, vec![later.display().to_string()], 2_700)
            .unwrap()[0]
            .id
            .clone();
        let earlier_id = jobs
            .create_imports(&media, vec![earlier.display().to_string()], 2_600)
            .unwrap()[0]
            .id
            .clone();
        let snapshot = jobs.snapshot(&media, 2_800).unwrap();

        assert_eq!(
            snapshot
                .iter()
                .map(|job| job.id.as_str())
                .collect::<Vec<_>>(),
            [earlier_id.as_str(), later_id.as_str()]
        );
        assert!(snapshot.iter().all(|job| job.id.starts_with("job-")));
        assert!(snapshot.iter().all(|job| job.id.parse::<u64>().is_err()));

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restart_keeps_a_missing_external_source_visible_but_never_reauthorizes_it() {
        let dir = temp_dir("restart-missing");
        let database = dir.join("jobs.sqlite3");
        let source = dir.join("moved.wav");
        fs::write(&source, b"RIFF-missing-after-restart").unwrap();
        let original_id = {
            let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
            let media = MediaOwner::new();
            let id = jobs
                .create_imports(&media, vec![source.display().to_string()], 3_000)
                .unwrap()[0]
                .id
                .clone();
            drop(media);
            id
        };
        fs::remove_file(&source).unwrap();

        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let snapshot = jobs.snapshot(&media, 3_001).unwrap();

        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, original_id);
        assert_eq!(snapshot[0].status, RecordingJobStatus::Failed);
        assert_eq!(snapshot[0].error.as_deref(), Some("SOURCE_MISSING"));
        assert_eq!(snapshot[0].source_path, None);
        assert_eq!(snapshot[0].playback_path, None);

        drop(media);
        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn product_bound_counts_existing_recoverable_jobs_across_create_calls() {
        let dir = temp_dir("product-bound");
        let paths = (0..MAX_RECORDING_JOBS)
            .map(|index| {
                let source = dir.join(format!("recording-{index:03}.wav"));
                fs::write(&source, b"RIFF-bound-fixture").unwrap();
                source.display().to_string()
            })
            .collect::<Vec<_>>();
        let overflow = dir.join("overflow.wav");
        fs::write(&overflow, b"RIFF-overflow-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        assert_eq!(
            jobs.create_imports(&media, paths, 4_000).unwrap().len(),
            MAX_RECORDING_JOBS
        );
        let admissions_before_overflow = media.active_admission_count_for_test();
        let registry_before_overflow = fs::read(&jobs.registry_path).unwrap();
        let error = jobs
            .create_imports(&media, vec![overflow.display().to_string()], 4_001)
            .unwrap_err();

        assert_eq!(error.code, "JOB_LIMIT_EXCEEDED");
        assert_eq!(
            media.active_admission_count_for_test(),
            admissions_before_overflow
        );
        assert_eq!(
            fs::read(&jobs.registry_path).unwrap(),
            registry_before_overflow
        );
        assert_eq!(
            jobs.snapshot(&media, 4_002).unwrap().len(),
            MAX_RECORDING_JOBS
        );
        assert_eq!(
            media.active_admission_count_for_test(),
            admissions_before_overflow
        );

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn snapshot_expires_pending_jobs_after_seven_days_without_touching_the_source() {
        let dir = temp_dir("pending-expiry");
        let source = dir.join("old.wav");
        fs::write(&source, b"RIFF-expiry-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let created = jobs
            .create_imports(&media, vec![source.display().to_string()], 5_000)
            .unwrap();
        let owned_spool = dir.join("remote-jobs").join(&created[0].id);
        fs::create_dir_all(&owned_spool).unwrap();
        fs::write(owned_spool.join("private.pcm"), b"private copy").unwrap();

        let snapshot = jobs
            .snapshot(&media, 5_000 + PENDING_JOB_LIFETIME_MS)
            .unwrap();

        assert_eq!(snapshot[0].status, RecordingJobStatus::Failed);
        assert_eq!(snapshot[0].error.as_deref(), Some("PENDING_EXPIRED"));
        assert_eq!(snapshot[0].source_path, None);
        assert_eq!(snapshot[0].playback_path, None);
        assert!(source.is_file(), "external source must never be deleted");
        assert!(
            !owned_spool.exists(),
            "expired jobs must delete Yap's private source copy"
        );
        let retried = jobs
            .retry(
                &media,
                &snapshot[0].id,
                5_001 + PENDING_JOB_LIFETIME_MS,
                || {},
            )
            .unwrap();
        assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
        assert_eq!(
            jobs.snapshot(&media, 5_001 + PENDING_JOB_LIFETIME_MS)
                .unwrap()[0]
                .status,
            RecordingJobStatus::QueuedServer
        );

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn snapshot_cancels_expired_active_remote_work_and_deletes_only_the_owned_spool() {
        let dir = temp_dir("active-remote-expiry");
        let source = dir.join("active.wav");
        fs::write(&source, b"RIFF-active-expiry-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let created = jobs
            .create_imports(&media, vec![source.display().to_string()], 6_000)
            .unwrap();
        let job_id = &created[0].id;
        jobs.ledger
            .transition(job_id, RecordingJobStatus::Preprocessing, 6_001)
            .unwrap();
        let owned_spool = dir.join("remote-jobs").join(job_id);
        fs::create_dir_all(&owned_spool).unwrap();
        fs::write(owned_spool.join("private.pcm"), b"private copy").unwrap();

        assert!(jobs
            .snapshot(&media, 6_000 + PENDING_JOB_LIFETIME_MS)
            .unwrap()
            .is_empty());
        let expired = jobs.ledger.get_job(job_id).unwrap().unwrap();
        assert_eq!(expired.status, RecordingJobStatus::Cancelled);
        assert!(expired.cancellation_requested);
        assert!(source.is_file(), "external source must never be deleted");
        assert!(
            !owned_spool.exists(),
            "expired active work must delete Yap's private source copy"
        );

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancellation_and_dismissal_delete_only_yap_owned_remote_spools() {
        let dir = temp_dir("remote-terminal-cleanup");
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        let cancel_source = dir.join("cancel.wav");
        fs::write(&cancel_source, b"RIFF-cancel-fixture").unwrap();
        let cancelled = jobs
            .create_imports(&media, vec![cancel_source.display().to_string()], 7_000)
            .unwrap();
        let cancel_spool = dir.join("remote-jobs").join(&cancelled[0].id);
        fs::create_dir_all(&cancel_spool).unwrap();
        fs::write(cancel_spool.join("private.pcm"), b"private copy").unwrap();
        jobs.cancel(&media, &cancelled[0].id, 7_001, || {}).unwrap();
        assert!(cancel_source.is_file());
        assert!(!cancel_spool.exists());

        let dismiss_source = dir.join("dismiss.wav");
        fs::write(&dismiss_source, b"RIFF-dismiss-fixture").unwrap();
        let dismissed = jobs
            .create_imports(&media, vec![dismiss_source.display().to_string()], 7_100)
            .unwrap();
        jobs.ledger
            .fail_source_validation(&dismissed[0].id, "TEST_FAILED", 7_101)
            .unwrap();
        let dismiss_spool = dir.join("remote-jobs").join(&dismissed[0].id);
        fs::create_dir_all(&dismiss_spool).unwrap();
        fs::write(dismiss_spool.join("private.pcm"), b"private copy").unwrap();
        jobs.dismiss(&media, &dismissed[0].id, 7_102, || {})
            .unwrap();
        assert!(dismiss_source.is_file());
        assert!(!dismiss_spool.exists());

        let retry_source = dir.join("retry.wav");
        fs::write(&retry_source, b"RIFF-retry-fixture").unwrap();
        let retried = jobs
            .create_imports(&media, vec![retry_source.display().to_string()], 7_200)
            .unwrap();
        jobs.ledger
            .fail_source_validation(&retried[0].id, "TEST_FAILED", 7_201)
            .unwrap();
        let retry_spool = dir.join("remote-jobs").join(&retried[0].id);
        fs::create_dir_all(&retry_spool).unwrap();
        fs::write(retry_spool.join("private.pcm"), b"private copy").unwrap();
        jobs.retry(&media, &retried[0].id, 7_202, || {}).unwrap();
        assert!(retry_source.is_file());
        assert!(!retry_spool.exists());

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dismissing_failed_jobs_preserves_provenance_and_frees_capacity_after_restart() {
        let dir = temp_dir("dismiss-capacity");
        let database = dir.join("jobs.sqlite3");
        let paths = (0..MAX_RECORDING_JOBS)
            .map(|index| {
                let source = dir.join(format!("failed-{index:03}.wav"));
                fs::write(&source, b"RIFF-failed-fixture").unwrap();
                source
            })
            .collect::<Vec<_>>();
        let replacement = dir.join("replacement.wav");
        fs::write(&replacement, b"RIFF-replacement-fixture").unwrap();

        {
            let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
            let media = MediaOwner::new();
            let created = jobs
                .create_imports(
                    &media,
                    paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect(),
                    5_500,
                )
                .unwrap();
            for (index, job) in created.iter().enumerate() {
                jobs.ledger
                    .fail_source_validation(&job.id, "TEST_FAILED", 5_600 + index as u64)
                    .unwrap();
                jobs.dismiss(&media, &job.id, 5_900 + index as u64, || {})
                    .unwrap();
            }

            assert!(jobs.snapshot(&media, 6_200).unwrap().is_empty());
            assert_eq!(media.active_admission_count_for_test(), 0);
            let durable = jobs.ledger.list_jobs().unwrap();
            assert_eq!(durable.len(), MAX_RECORDING_JOBS);
            assert!(durable.iter().all(|job| {
                job.status == RecordingJobStatus::Cancelled
                    && job.source_path.is_some()
                    && job.error_code.as_deref() == Some("TEST_FAILED")
                    && job.cancellation_requested
            }));
        }

        assert!(paths.iter().all(|path| path.is_file()));
        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        assert!(jobs.snapshot(&media, 6_300).unwrap().is_empty());
        let imported = jobs
            .create_imports(&media, vec![replacement.display().to_string()], 6_301)
            .unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].status, RecordingJobStatus::QueuedServer);

        drop(media);
        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn more_than_five_hundred_terminal_imports_do_not_exhaust_job_path_authority() {
        let dir = temp_dir("terminal-authority-cycles");
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let mut sources = Vec::new();

        for index in 0..=500 {
            let source = dir.join(format!("terminal-{index:03}.wav"));
            fs::write(&source, b"RIFF-terminal-authority-fixture").unwrap();
            let created = jobs
                .create_imports(
                    &media,
                    vec![source.display().to_string()],
                    6_500 + index as u64 * 3,
                )
                .unwrap();
            assert_eq!(created[0].status, RecordingJobStatus::QueuedServer);
            if index % 2 == 0 {
                jobs.cancel(&media, &created[0].id, 6_501 + index as u64 * 3, || {})
                    .unwrap();
            } else {
                jobs.ledger
                    .fail_source_validation(&created[0].id, "TEST_FAILED", 6_501 + index as u64 * 3)
                    .unwrap();
                jobs.dismiss(&media, &created[0].id, 6_502 + index as u64 * 3, || {})
                    .unwrap();
            }
            sources.push(source);
        }

        let final_source = dir.join("still-authorized.wav");
        fs::write(&final_source, b"RIFF-final-authority-fixture").unwrap();
        let final_import = jobs
            .create_imports(&media, vec![final_source.display().to_string()], 8_100)
            .unwrap();

        assert_eq!(final_import[0].status, RecordingJobStatus::QueuedServer);
        assert!(sources.iter().all(|source| source.is_file()));

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn terminal_job_authority_is_removed_without_harming_general_authority_or_bytes() {
        let dir = temp_dir("terminal-authority-removal");
        let general_registry = dir.join("recording-playback-registry.json");
        let cancelled_source = dir.join("cancelled.wav");
        let dismissed_source = dir.join("dismissed.wav");
        let general_source = dir.join("general.wav");
        for source in [&cancelled_source, &dismissed_source, &general_source] {
            fs::write(source, b"RIFF-terminal-authority-fixture").unwrap();
        }
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        let cancelled = jobs
            .create_imports(&media, vec![cancelled_source.display().to_string()], 8_200)
            .unwrap();
        jobs.cancel(&media, &cancelled[0].id, 8_201, || {}).unwrap();
        assert!(crate::file_actions::openable_app_path_from_registries(
            cancelled_source.display().to_string(),
            &general_registry,
            &jobs.registry_path,
            &jobs.owned_dir,
        )
        .is_err());

        let dismissed = jobs
            .create_imports(&media, vec![dismissed_source.display().to_string()], 8_202)
            .unwrap();
        jobs.ledger
            .fail_source_validation(&dismissed[0].id, "TEST_FAILED", 8_203)
            .unwrap();
        jobs.dismiss(&media, &dismissed[0].id, 8_204, || {})
            .unwrap();
        assert!(crate::file_actions::openable_app_path_from_registries(
            dismissed_source.display().to_string(),
            &general_registry,
            &jobs.registry_path,
            &jobs.owned_dir,
        )
        .is_err());

        let general = jobs
            .create_imports(&media, vec![general_source.display().to_string()], 8_205)
            .unwrap();
        crate::file_actions::register_general_playback_path_at_for_test(
            general_source.display().to_string(),
            &general_registry,
            &jobs.owned_dir,
        )
        .unwrap();
        jobs.ledger
            .fail_source_validation(&general[0].id, "TEST_FAILED", 8_206)
            .unwrap();
        jobs.dismiss(&media, &general[0].id, 8_207, || {}).unwrap();
        assert_eq!(
            crate::file_actions::openable_app_path_from_registries(
                general_source.display().to_string(),
                &general_registry,
                &jobs.registry_path,
                &jobs.owned_dir,
            )
            .unwrap(),
            general_source.canonicalize().unwrap()
        );
        assert!([&cancelled_source, &dismissed_source, &general_source]
            .iter()
            .all(|source| source.is_file()));

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restart_snapshot_prunes_job_authority_left_by_a_terminal_commit() {
        let dir = temp_dir("restart-authority-prune");
        let database = dir.join("jobs.sqlite3");
        let general_registry = dir.join("recording-playback-registry.json");
        let source = dir.join("stale.wav");
        fs::write(&source, b"RIFF-stale-authority-fixture").unwrap();

        {
            let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
            let media = MediaOwner::new();
            let created = jobs
                .create_imports(&media, vec![source.display().to_string()], 8_300)
                .unwrap();
            assert!(crate::file_actions::openable_app_path_from_registries(
                source.display().to_string(),
                &general_registry,
                &jobs.registry_path,
                &jobs.owned_dir,
            )
            .is_ok());
            jobs.ledger
                .request_cancellation(&created[0].id, 8_301)
                .unwrap();
        }

        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        assert!(jobs.snapshot(&media, 8_302).unwrap().is_empty());
        assert!(crate::file_actions::openable_app_path_from_registries(
            source.display().to_string(),
            &general_registry,
            &jobs.registry_path,
            &jobs.owned_dir,
        )
        .is_err());
        assert!(source.is_file());

        drop(media);
        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn terminal_registry_cleanup_failure_does_not_hide_the_committed_transition() {
        let dir = temp_dir("terminal-cleanup-failure");
        let source = dir.join("cleanup.wav");
        fs::write(&source, b"RIFF-cleanup-failure-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let created = jobs
            .create_imports(&media, vec![source.display().to_string()], 8_400)
            .unwrap();
        fs::remove_file(&jobs.registry_path).unwrap();
        fs::create_dir(&jobs.registry_path).unwrap();

        let cancelled = mutate_then_notify(
            || jobs.cancel(&media, &created[0].id, 8_401, || {}),
            || {
                assert_eq!(
                    jobs.ledger.get_job(&created[0].id).unwrap().unwrap().status,
                    RecordingJobStatus::Cancelled
                );
            },
        )
        .unwrap();

        assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
        assert!(jobs.snapshot(&media, 8_402).unwrap().is_empty());
        assert!(source.is_file());

        fs::remove_dir(&jobs.registry_path).unwrap();
        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancellation_and_retry_follow_ledger_legality_and_preserve_external_files() {
        let dir = temp_dir("cancel-retry");
        let cancel_source = dir.join("cancel.wav");
        let retry_source = dir.join("retry.wav");
        fs::write(&cancel_source, b"RIFF-cancel-fixture").unwrap();
        fs::write(&retry_source, b"RIFF-retry-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let cancel_id = jobs
            .create_imports(&media, vec![cancel_source.display().to_string()], 6_000)
            .unwrap()[0]
            .id
            .clone();
        let retry_id = jobs
            .create_imports(&media, vec![retry_source.display().to_string()], 6_001)
            .unwrap()[0]
            .id
            .clone();
        let admissions_before_illegal_dismiss = media.active_admission_count_for_test();
        assert!(jobs.dismiss(&media, &cancel_id, 6_002, || {}).is_err());
        assert_eq!(
            jobs.ledger.get_job(&cancel_id).unwrap().unwrap().status,
            RecordingJobStatus::QueuedServer
        );
        assert_eq!(
            media.active_admission_count_for_test(),
            admissions_before_illegal_dismiss
        );
        jobs.ledger
            .fail_source_validation(&retry_id, "SOURCE_UNSAFE", 6_003)
            .unwrap();

        let cancelled = jobs.cancel(&media, &cancel_id, 6_004, || {}).unwrap();
        let retried = jobs.retry(&media, &retry_id, 6_005, || {}).unwrap();

        assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
        assert!(cancel_source.is_file());
        assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
        assert!(jobs.cancel(&media, &cancel_id, 6_006, || {}).is_err());
        let admissions_before_illegal_retry = media.active_admission_count_for_test();
        let registry_before_illegal_retry = fs::read(&jobs.registry_path).unwrap();
        assert!(jobs.retry(&media, &cancel_id, 6_007, || {}).is_err());
        assert_eq!(
            media.active_admission_count_for_test(),
            admissions_before_illegal_retry
        );
        assert_eq!(
            fs::read(&jobs.registry_path).unwrap(),
            registry_before_illegal_retry
        );
        let recreated = jobs
            .create_imports(&media, vec![cancel_source.display().to_string()], 6_008)
            .unwrap();
        assert_ne!(recreated[0].id, cancel_id);
        assert_eq!(recreated[0].status, RecordingJobStatus::QueuedServer);

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restart_rejects_a_source_replaced_by_a_reparse_point() {
        let dir = temp_dir("restart-reparse");
        let database = dir.join("jobs.sqlite3");
        let source = dir.join("source.wav");
        let target_dir = dir.join("reparse-target");
        fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("target.wav");
        fs::write(&source, b"RIFF-original-fixture").unwrap();
        fs::write(&target, b"RIFF-target-fixture").unwrap();
        {
            let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
            let media = MediaOwner::new();
            jobs.create_imports(&media, vec![source.display().to_string()], 7_000)
                .unwrap();
        }
        fs::remove_file(&source).unwrap();
        create_reparse_point(&target, &source).expect(
            "reparse fixture creation failed; tests require file symlinks or NTFS directory junctions",
        );
        let link_metadata = fs::symlink_metadata(&source).unwrap();
        assert!(
            link_metadata.file_type().is_symlink()
                || crate::file_actions::metadata_is_reparse_point_for_test(&link_metadata),
            "fixture must be a symlink or Windows reparse point"
        );

        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let snapshot = jobs.snapshot(&media, 7_001).unwrap();

        assert_eq!(snapshot[0].status, RecordingJobStatus::Failed);
        assert_eq!(snapshot[0].error.as_deref(), Some("SOURCE_UNSAFE"));
        assert_eq!(snapshot[0].source_path, None);
        assert_eq!(snapshot[0].playback_path, None);

        remove_reparse_point(&source).unwrap();
        drop(media);
        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_and_cancel_notifications_observe_committed_ledger_state() {
        let dir = temp_dir("event-after-commit");
        let source = dir.join("event.wav");
        fs::write(&source, b"RIFF-event-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();

        let created = mutate_then_notify(
            || jobs.create_imports(&media, vec![source.display().to_string()], 8_000),
            || {
                assert_eq!(jobs.ledger.list_jobs().unwrap().len(), 1);
            },
        )
        .unwrap();
        let job_id = created[0].id.clone();
        mutate_then_notify(
            || jobs.cancel(&media, &job_id, 8_001, || {}),
            || {
                assert_eq!(
                    jobs.ledger.get_job(&job_id).unwrap().unwrap().status,
                    RecordingJobStatus::Cancelled
                );
            },
        )
        .unwrap();

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn accepted_retry_notifies_only_after_atomic_preflight_returns_to_server_queue() {
        let dir = temp_dir("accepted-retry-event");
        let source = dir.join("accepted.wav");
        fs::write(&source, b"RIFF-accepted-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let selected_source = jobs.validate_source(&source).unwrap();
        crate::file_actions::register_native_selected_recording_job_source_at(
            &selected_source,
            &jobs.selection_registry_path,
            &jobs.owned_dir,
        )
        .unwrap();
        jobs.ledger
            .insert_job(&NewRecordingJob {
                job_id: "job-accepted".into(),
                session_mode: SessionMode::Meeting,
                session_origin: SessionOrigin::ImportedFile,
                source_path: Some(source.canonicalize().unwrap()),
                source_ownership: SourceOwnership::External,
                output_path: None,
                display_name: "accepted.wav".into(),
                status: RecordingJobStatus::Accepted,
                route: Some(RecordingRoute::ServerBatch),
                attempt_count: 0,
                next_attempt_at_ms: None,
                cancellation_requested: false,
                capture_commit_path: None,
                capture_manifest_sha256: None,
                error_code: None,
                error_message: None,
                created_at_ms: 8_500,
                updated_at_ms: 8_500,
                expires_at_ms: Some(8_500 + PENDING_JOB_LIFETIME_MS),
            })
            .unwrap();

        let retried = jobs
            .retry(&media, "job-accepted", 8_501, || {
                assert_eq!(
                    jobs.ledger.get_job("job-accepted").unwrap().unwrap().status,
                    RecordingJobStatus::QueuedServer
                );
            })
            .unwrap();

        assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
        let renewed_expiry = 8_501 + PENDING_JOB_LIFETIME_MS;
        assert_eq!(
            jobs.ledger
                .get_job("job-accepted")
                .unwrap()
                .unwrap()
                .expires_at_ms,
            Some(renewed_expiry)
        );
        assert_eq!(
            jobs.snapshot(&media, renewed_expiry - 1).unwrap()[0].status,
            RecordingJobStatus::QueuedServer
        );
        let at_boundary = jobs.snapshot(&media, renewed_expiry).unwrap();
        assert_eq!(at_boundary[0].status, RecordingJobStatus::Failed);
        assert_eq!(at_boundary[0].error.as_deref(), Some("PENDING_EXPIRED"));
        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn authority_failed_retry_stays_capability_free_until_a_second_explicit_retry() {
        let dir = temp_dir("retry-admission-failure");
        let database = dir.join("jobs.sqlite3");
        let general_registry = dir.join("recording-playback-registry.json");
        let source = dir.join("retry.wav");
        let source_bytes = b"RIFF-retry-admission-fixture";
        fs::write(&source, source_bytes).unwrap();
        let canonical_source = source.canonicalize().unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let job_id = jobs
            .create_imports(&media, vec![source.display().to_string()], 8_700)
            .unwrap()[0]
            .id
            .clone();
        jobs.ledger
            .fail_source_validation(&job_id, "INITIAL_FAILURE", 8_701)
            .unwrap();
        jobs.inject_projection_failures_for_test(vec![command_error(
            "PLAYBACK_AUTHORITY_FAILED",
            "injected retry admission failure",
        )]);
        let authority_denied_before_event_snapshot = Cell::new(false);
        let authority_denied_after_event_snapshot = Cell::new(false);
        let event_snapshot = RefCell::new(None);

        let retried = mutate_then_notify(
            || jobs.retry(&media, &job_id, 8_702, || {}),
            || {
                authority_denied_before_event_snapshot.set(open_and_reveal_are_denied(
                    &jobs,
                    &source,
                    &general_registry,
                ));
                let snapshot = jobs.snapshot(&media, 8_703).unwrap();
                authority_denied_after_event_snapshot.set(open_and_reveal_are_denied(
                    &jobs,
                    &source,
                    &general_registry,
                ));
                *event_snapshot.borrow_mut() = Some(snapshot);
            },
        )
        .unwrap();
        let event_snapshot = event_snapshot.into_inner().unwrap();
        let committed = jobs.ledger.get_job(&job_id).unwrap().unwrap();

        assert_eq!(committed.status, RecordingJobStatus::Failed);
        assert_eq!(committed.attempt_count, 1);
        assert_eq!(
            committed.error_code.as_deref(),
            Some("PLAYBACK_AUTHORITY_FAILED")
        );
        assert_eq!(
            committed.source_path.as_deref(),
            Some(canonical_source.as_path())
        );
        assert!(capability_free_failed(&retried));
        assert!(capability_free_failed(&event_snapshot[0]));
        assert!(authority_denied_before_event_snapshot.get());
        assert!(authority_denied_after_event_snapshot.get());
        assert_eq!(fs::read(&source).unwrap(), source_bytes);

        drop(media);
        drop(jobs);

        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let restart_snapshot = jobs.snapshot(&media, 8_704).unwrap();
        assert!(capability_free_failed(&restart_snapshot[0]));
        assert!(open_and_reveal_are_denied(
            &jobs,
            &source,
            &general_registry
        ));
        let restarted = jobs.ledger.get_job(&job_id).unwrap().unwrap();
        assert_eq!(restarted.attempt_count, 1);
        assert_eq!(
            restarted.source_path.as_deref(),
            Some(canonical_source.as_path())
        );

        let second_retry = jobs.retry(&media, &job_id, 8_705, || {}).unwrap();
        assert_eq!(second_retry.status, RecordingJobStatus::QueuedServer);
        assert_eq!(
            second_retry.source_path.as_deref(),
            canonical_source.to_str()
        );
        assert!(second_retry.playback_path.is_some());
        assert_eq!(
            jobs.ledger.get_job(&job_id).unwrap().unwrap().attempt_count,
            2
        );
        assert_eq!(
            crate::file_actions::openable_app_path_from_registries(
                source.display().to_string(),
                &general_registry,
                &jobs.registry_path,
                &jobs.owned_dir,
            )
            .unwrap(),
            canonical_source
        );
        assert_eq!(fs::read(&source).unwrap(), source_bytes);

        drop(media);
        drop(jobs);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    fn create_reparse_point(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_reparse_point(target: &Path, link: &Path) -> std::io::Result<()> {
        let target_dir = target.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "target has no parent")
        })?;
        let output = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target_dir)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }

    #[cfg(unix)]
    fn remove_reparse_point(link: &Path) -> std::io::Result<()> {
        fs::remove_file(link)
    }

    #[cfg(windows)]
    fn remove_reparse_point(link: &Path) -> std::io::Result<()> {
        fs::remove_dir(link)
    }

    fn capability_free_failed(view: &RecordingJobView) -> bool {
        view.status == RecordingJobStatus::Failed
            && view.source_path.is_none()
            && view.playback_path.is_none()
    }

    fn open_and_reveal_are_denied(
        jobs: &RecordingJobs,
        source: &Path,
        general_registry: &Path,
    ) -> bool {
        let authorization_denied = || {
            crate::file_actions::openable_app_path_from_registries(
                source.display().to_string(),
                general_registry,
                &jobs.registry_path,
                &jobs.owned_dir,
            )
            .is_err()
        };
        authorization_denied() && authorization_denied()
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "yap-job-commands-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_pcm_wav(path: &Path, pcm: &[u8]) {
        let mut file = fs::File::create(path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(36_u32 + pcm.len() as u32).to_le_bytes())
            .unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16_u32.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&16_000_u32.to_le_bytes()).unwrap();
        file.write_all(&32_000_u32.to_le_bytes()).unwrap();
        file.write_all(&2_u16.to_le_bytes()).unwrap();
        file.write_all(&16_u16.to_le_bytes()).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&(pcm.len() as u32).to_le_bytes()).unwrap();
        file.write_all(pcm).unwrap();
        file.sync_all().unwrap();
    }
}
