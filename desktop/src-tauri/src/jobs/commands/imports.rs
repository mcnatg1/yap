use super::{
    command_error, mint_job_id, source_error, JobCommandError, RecordingJobs, MAX_RECORDING_JOBS,
    PENDING_JOB_LIFETIME_MS, PHASE5_REMOTE_IMPORT_EXTENSIONS,
};
use crate::{
    commands::media_protocol::MediaOwner,
    jobs::{
        NewRecordingJob, RecordingJobStatus, RecordingJobView, RecordingRoute, SessionMode,
        SessionOrigin, SourceOwnership,
    },
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

impl RecordingJobs {
    pub(super) fn create_imports<P: AsRef<Path>>(
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
}
