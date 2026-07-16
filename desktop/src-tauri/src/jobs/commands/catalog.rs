use super::super::remote;
use super::{
    CompletedRemoteTranscript, CompletedRemoteTranscriptCatalog, JobCommandError, RecordingJobs,
};
use crate::{
    jobs::{RecordingJobStatus, RecordingRoute},
    server_connector::batch::CreateRecordingJobRequest,
};

impl RecordingJobs {
    pub(crate) fn completed_remote_transcripts(
        &self,
    ) -> Result<CompletedRemoteTranscriptCatalog, JobCommandError> {
        let mut sessions = Vec::new();
        let mut omitted_invalid_result = false;
        for record in self.ledger().list_jobs()?.into_iter().filter(|record| {
            matches!(
                record.status,
                RecordingJobStatus::Complete | RecordingJobStatus::Partial
            ) && record.route == Some(RecordingRoute::ServerBatch)
        }) {
            let verified = (|| {
                let output_path = record.output_path.as_deref().ok_or(())?;
                let source_path = record.source_path.as_deref().ok_or(())?;
                let prepared = self
                    .ledger()
                    .get_prepared_remote_job(&record.job_id)
                    .map_err(|_| ())?
                    .ok_or(())?;
                let request =
                    CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json)
                        .map_err(|_| ())?;
                let verified = remote::read_published_remote_transcript(
                    output_path,
                    self.remote_jobs_directory(),
                )
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
}
