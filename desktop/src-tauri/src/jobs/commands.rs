use crate::{
    commands::media_protocol::MediaOwner,
    file_actions::{RecordingJobSourceAdmission, RecordingJobSourceError},
    jobs::{
        JobLedger, JobLedgerError, NewRecordingJob, RecordingJobStatus, RecordingJobView,
        RecordingRoute, SessionMode, SessionOrigin, SourceOwnership,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::Emitter;

const PENDING_JOB_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_RECORDING_JOBS: usize = 200;
static NEXT_JOB_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCommandError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyQueueImport {
    pub schema_version: u32,
    pub jobs: Vec<LegacyQueueJob>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyQueueJob {
    pub id: u64,
    pub path: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportAcknowledgement {
    pub legacy_id: u64,
    pub job_id: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportRejection {
    pub legacy_id: u64,
    pub code: String,
    pub message: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportResult {
    pub accepted: Vec<LegacyImportAcknowledgement>,
    pub duplicates: Vec<LegacyImportAcknowledgement>,
    pub rejected: Vec<LegacyImportRejection>,
}

impl From<JobLedgerError> for JobCommandError {
    fn from(error: JobLedgerError) -> Self {
        Self {
            code: "JOB_LEDGER_ERROR".into(),
            message: error.to_string(),
        }
    }
}

pub(crate) struct RecordingJobs {
    ledger: JobLedger,
    mutation: Mutex<()>,
    owned_dir: PathBuf,
    registry_path: PathBuf,
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
pub(crate) fn recording_jobs_create_imports(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    jobs: tauri::State<'_, RecordingJobs>,
    media: tauri::State<'_, MediaOwner>,
    paths: Vec<String>,
) -> Result<Vec<RecordingJobView>, JobCommandError> {
    ensure_main(&window)?;
    mutate_then_notify(
        || jobs.create_imports(&media, paths, now_ms()?),
        || emit_jobs_changed(&app),
    )
}

#[tauri::command]
pub(crate) fn recording_jobs_import_legacy(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    jobs: tauri::State<'_, RecordingJobs>,
    media: tauri::State<'_, MediaOwner>,
    payload: LegacyQueueImport,
) -> Result<LegacyImportResult, JobCommandError> {
    ensure_main(&window)?;
    mutate_then_notify(
        || jobs.import_legacy(&media, payload, now_ms()?),
        || emit_jobs_changed(&app),
    )
}

#[tauri::command]
pub(crate) fn recording_job_cancel(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    jobs: tauri::State<'_, RecordingJobs>,
    job_id: String,
) -> Result<RecordingJobView, JobCommandError> {
    ensure_main(&window)?;
    mutate_then_notify(
        || jobs.cancel(&job_id, now_ms()?, || {}),
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
    if let Err(error) = app.emit("recording-jobs-changed", ()) {
        crate::stt::log_yap(&format!(
            "recording jobs event failed after commit: {error}"
        ));
    }
}

impl RecordingJobs {
    pub fn open_default() -> Result<Self, JobCommandError> {
        Ok(Self {
            ledger: JobLedger::open_default()?,
            mutation: Mutex::new(()),
            owned_dir: crate::live::recordings::recordings_dir(),
            registry_path: crate::paths::app_data_dir().join("recording-playback-registry.json"),
        })
    }

    #[cfg(test)]
    fn from_ledger(ledger: JobLedger, authority_dir: &Path) -> Self {
        let owned_dir = authority_dir.join("owned-live-recordings");
        std::fs::create_dir_all(&owned_dir).expect("prepare test owned directory");
        Self {
            ledger,
            mutation: Mutex::new(()),
            owned_dir,
            registry_path: authority_dir.join("recording-playback-registry.json"),
        }
    }

    fn create_imports(
        &self,
        media: &MediaOwner,
        paths: Vec<String>,
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
        let sources = paths
            .iter()
            .map(|path| self.authorize_source(media, Path::new(path)))
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

        let mut created = Vec::with_capacity(sources.len());
        for source in sources {
            if let Some(existing) = self
                .ledger
                .find_recoverable_imported_job_by_source(&source.canonical_path)?
            {
                created.push(project_with_admission(existing, source));
                continue;
            }
            let id = mint_job_id(&source.canonical_path, now_ms);
            let record = self.ledger.insert_job(&NewRecordingJob {
                job_id: id,
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
            })?;
            created.push(project_with_admission(record, source));
        }
        Ok(created)
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
        let mut views = Vec::new();
        for record in self.ledger.list_recoverable_jobs()? {
            if record.error_code.as_deref() == Some("PENDING_EXPIRED") {
                let mut view = RecordingJobView::from_record(&record);
                view.source_path = None;
                view.playback_path = None;
                views.push(view);
                continue;
            }
            match self.project_with_playback(record.clone(), media) {
                Ok(view) => views.push(view),
                Err(error) if error.code == "SOURCE_MISSING" || error.code == "SOURCE_UNSAFE" => {
                    let failed =
                        self.ledger
                            .fail_source_validation(&record.job_id, &error.code, now_ms)?;
                    let mut view = RecordingJobView::from_record(&failed);
                    view.source_path = None;
                    view.playback_path = None;
                    views.push(view);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(views)
    }

    fn cancel(
        &self,
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
        let source = current.source_path.as_deref().ok_or_else(|| {
            command_error("SOURCE_UNSAFE", "Imported recording has no source path.")
        })?;
        let source = self.authorize_source(media, source)?;

        let (record, changed) = match current.status {
            RecordingJobStatus::Accepted => {
                (self.ledger.accept_to_queued_server(job_id, now_ms)?, true)
            }
            RecordingJobStatus::BlockedSetupRequired
            | RecordingJobStatus::BlockedServerUnavailable
            | RecordingJobStatus::BlockedSignInRequired
            | RecordingJobStatus::Failed => (
                self.ledger.retry_to_queued_server(
                    job_id,
                    now_ms,
                    now_ms.checked_add(PENDING_JOB_LIFETIME_MS),
                )?,
                true,
            ),
            RecordingJobStatus::QueuedServer => (current, false),
            _ => {
                return Err(command_error(
                    "INVALID_JOB_TRANSITION",
                    format!("Recording job {job_id:?} cannot be retried from its current state."),
                ));
            }
        };
        let mut view = RecordingJobView::from_record(&record);
        view.source_path = Some(source.canonical_path.display().to_string());
        view.playback_path = Some(source.playback_path);
        drop(mutation);
        if changed {
            notify();
        }
        Ok(view)
    }

    fn import_legacy(
        &self,
        media: &MediaOwner,
        payload: LegacyQueueImport,
        now_ms: u64,
    ) -> Result<LegacyImportResult, JobCommandError> {
        if payload.schema_version != 1 {
            return Err(command_error(
                "LEGACY_SCHEMA_UNSUPPORTED",
                format!(
                    "Legacy recording queue schema {} is not supported.",
                    payload.schema_version
                ),
            ));
        }
        if payload.jobs.len() > MAX_RECORDING_JOBS {
            return Err(command_error(
                "JOB_LIMIT_EXCEEDED",
                format!("Legacy import accepts at most {MAX_RECORDING_JOBS} rows."),
            ));
        }

        let _mutation = self.mutation.lock().map_err(|_| {
            command_error(
                "JOB_STATE_UNAVAILABLE",
                "Recording job state is unavailable.",
            )
        })?;
        let mut result = LegacyImportResult {
            accepted: Vec::new(),
            duplicates: Vec::new(),
            rejected: Vec::new(),
        };
        let mut recoverable_count = self.ledger.list_recoverable_jobs()?.len();

        for (index, legacy) in payload.jobs.into_iter().enumerate() {
            let job_id = legacy_job_id(legacy.id, &legacy.path);
            if let Some(existing) = self.ledger.get_job(&job_id)? {
                result.duplicates.push(LegacyImportAcknowledgement {
                    legacy_id: legacy.id,
                    job_id: existing.job_id,
                });
                continue;
            }
            let admission = match self.authorize_source(media, Path::new(&legacy.path)) {
                Ok(admission) => admission,
                Err(error) => {
                    result.rejected.push(LegacyImportRejection {
                        legacy_id: legacy.id,
                        code: error.code,
                        message: error.message,
                    });
                    continue;
                }
            };
            if let Some(existing) = self
                .ledger
                .find_recoverable_imported_job_by_source(&admission.canonical_path)?
            {
                result.duplicates.push(LegacyImportAcknowledgement {
                    legacy_id: legacy.id,
                    job_id: existing.job_id,
                });
                continue;
            }
            if recoverable_count >= MAX_RECORDING_JOBS {
                result.rejected.push(LegacyImportRejection {
                    legacy_id: legacy.id,
                    code: "JOB_LIMIT_EXCEEDED".into(),
                    message: format!("Yap accepts at most {MAX_RECORDING_JOBS} recording jobs."),
                });
                continue;
            }

            let row_now = now_ms.saturating_add(index as u64);
            let record = self.ledger.insert_job(&NewRecordingJob {
                job_id: job_id.clone(),
                session_mode: SessionMode::Meeting,
                session_origin: SessionOrigin::ImportedFile,
                source_path: Some(admission.canonical_path.clone()),
                source_ownership: SourceOwnership::External,
                output_path: None,
                display_name: admission
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
                created_at_ms: row_now,
                updated_at_ms: row_now,
                expires_at_ms: row_now.checked_add(PENDING_JOB_LIFETIME_MS),
            })?;
            recoverable_count += 1;
            result.accepted.push(LegacyImportAcknowledgement {
                legacy_id: legacy.id,
                job_id: record.job_id,
            });
        }

        Ok(result)
    }

    fn authorize_source(
        &self,
        media: &MediaOwner,
        path: &Path,
    ) -> Result<RecordingJobSourceAdmission, JobCommandError> {
        crate::file_actions::authorize_recording_job_source_at(
            path,
            media,
            &self.registry_path,
            &self.owned_dir,
        )
        .map_err(source_error)
    }

    fn project_with_playback(
        &self,
        record: crate::jobs::RecordingJobRecord,
        media: &MediaOwner,
    ) -> Result<RecordingJobView, JobCommandError> {
        let Some(source) = record.source_path.as_deref() else {
            return Ok(RecordingJobView::from_record(&record));
        };
        let admission = crate::file_actions::authorize_recording_job_source_at(
            source,
            media,
            &self.registry_path,
            &self.owned_dir,
        )
        .map_err(source_error)?;
        Ok(project_with_admission(record, admission))
    }
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

fn legacy_job_id(legacy_id: u64, path: &str) -> String {
    let normalized = if cfg!(windows) {
        path.replace('\\', "/").to_lowercase()
    } else {
        path.replace('\\', "/")
    };
    let mut hash = Sha256::new();
    hash.update(normalized.as_bytes());
    format!("legacy-{legacy_id}-{}", hex_prefix(&hash.finalize(), 16))
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

fn mutate_then_notify<T, E>(
    mutation: impl FnOnce() -> Result<T, E>,
    notify: impl FnOnce(),
) -> Result<T, E> {
    let value = mutation()?;
    notify();
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::media_protocol::MediaOwner, jobs::JobLedger};
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

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
        assert!(
            fs::read_to_string(dir.join("recording-playback-registry.json"))
                .unwrap()
                .contains("meeting.wav")
        );

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
        assert_eq!(jobs.snapshot(&media, 2_002).unwrap().len(), 1);

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
        let error = jobs
            .create_imports(&media, vec![overflow.display().to_string()], 4_001)
            .unwrap_err();

        assert_eq!(error.code, "JOB_LIMIT_EXCEEDED");
        assert_eq!(
            jobs.snapshot(&media, 4_002).unwrap().len(),
            MAX_RECORDING_JOBS
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
        jobs.create_imports(&media, vec![source.display().to_string()], 5_000)
            .unwrap();

        let snapshot = jobs
            .snapshot(&media, 5_000 + PENDING_JOB_LIFETIME_MS)
            .unwrap();

        assert_eq!(snapshot[0].status, RecordingJobStatus::Failed);
        assert_eq!(snapshot[0].error.as_deref(), Some("PENDING_EXPIRED"));
        assert_eq!(snapshot[0].source_path, None);
        assert_eq!(snapshot[0].playback_path, None);
        assert!(source.is_file(), "external source must never be deleted");
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
        jobs.ledger
            .fail_source_validation(&retry_id, "SOURCE_UNSAFE", 6_002)
            .unwrap();

        let cancelled = jobs.cancel(&cancel_id, 6_003, || {}).unwrap();
        let retried = jobs.retry(&media, &retry_id, 6_004, || {}).unwrap();

        assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
        assert!(cancel_source.is_file());
        assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
        assert!(jobs.cancel(&cancel_id, 6_005, || {}).is_err());
        assert!(jobs.retry(&media, &cancel_id, 6_006, || {}).is_err());
        let recreated = jobs
            .create_imports(&media, vec![cancel_source.display().to_string()], 6_007)
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
        let target = dir.join("target.wav");
        fs::write(&source, b"RIFF-original-fixture").unwrap();
        fs::write(&target, b"RIFF-target-fixture").unwrap();
        {
            let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
            let media = MediaOwner::new();
            jobs.create_imports(&media, vec![source.display().to_string()], 7_000)
                .unwrap();
        }
        fs::remove_file(&source).unwrap();
        if create_file_symlink(&target, &source).is_err() {
            fs::remove_dir_all(dir).unwrap();
            return;
        }

        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let snapshot = jobs.snapshot(&media, 7_001).unwrap();

        assert_eq!(snapshot[0].status, RecordingJobStatus::Failed);
        assert_eq!(snapshot[0].error.as_deref(), Some("SOURCE_UNSAFE"));
        assert_eq!(snapshot[0].source_path, None);
        assert_eq!(snapshot[0].playback_path, None);

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
            || jobs.cancel(&job_id, 8_001, || {}),
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
        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_import_is_bounded_acknowledged_and_idempotent_with_deterministic_ids() {
        let dir = temp_dir("legacy-import");
        let source = dir.join("legacy.wav");
        let missing = dir.join("missing.wav");
        fs::write(&source, b"RIFF-legacy-fixture").unwrap();
        let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
        let media = MediaOwner::new();
        let payload = LegacyQueueImport {
            schema_version: 1,
            jobs: vec![
                LegacyQueueJob {
                    id: 41,
                    path: source.display().to_string(),
                },
                LegacyQueueJob {
                    id: 42,
                    path: missing.display().to_string(),
                },
            ],
        };

        let first = jobs.import_legacy(&media, payload.clone(), 9_000).unwrap();
        assert_eq!(first.accepted.len(), 1);
        assert_eq!(first.duplicates.len(), 0);
        assert_eq!(first.rejected.len(), 1);
        assert!(first.accepted[0].job_id.starts_with("legacy-41-"));
        assert_eq!(first.rejected[0].legacy_id, 42);
        assert_eq!(first.rejected[0].code, "SOURCE_MISSING");

        fs::remove_file(&source).unwrap();
        let replay = jobs.import_legacy(&media, payload, 9_001).unwrap();
        assert_eq!(replay.accepted.len(), 0);
        assert_eq!(replay.duplicates.len(), 1);
        assert_eq!(replay.duplicates[0].job_id, first.accepted[0].job_id);
        assert_eq!(replay.rejected.len(), 1);
        assert_eq!(jobs.ledger.list_jobs().unwrap().len(), 1);

        let overflow = LegacyQueueImport {
            schema_version: 1,
            jobs: (0..=MAX_RECORDING_JOBS)
                .map(|id| LegacyQueueJob {
                    id: id as u64 + 1,
                    path: format!("C:/legacy-{id}.wav"),
                })
                .collect(),
        };
        assert_eq!(
            jobs.import_legacy(&media, overflow, 9_002)
                .unwrap_err()
                .code,
            "JOB_LIMIT_EXCEEDED"
        );

        drop(media);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
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
}
