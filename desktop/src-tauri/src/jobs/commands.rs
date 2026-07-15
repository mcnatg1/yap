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

    pub(crate) fn completed_remote_transcripts(
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
mod tests;
