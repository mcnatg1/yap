pub mod commands;
mod drain;
mod ledger;
mod migrations;
mod model;
mod remote;

pub use ledger::JobLedger;
pub use model::{
    DetachedRemoteCancellationRecord, JobChunkRecord, JobLedgerError, NewJobChunk,
    NewPreparedRemoteJob, NewRecordingJob, PreparedRemoteJobRecord, RecordingJobRecord,
    RecordingJobStatus, RecordingJobView, RecordingRoute, SessionMode, SessionOrigin,
    SourceOwnership,
};

pub(crate) use drain::RemoteJobDrain;

pub(crate) fn start_remote_job_drain(
    app: &tauri::AppHandle,
    lifecycle: &crate::runtime::DesktopLifecycle,
) -> std::io::Result<()> {
    drain::start(app, lifecycle)
}

fn remote_jobs_directory() -> std::path::PathBuf {
    crate::paths::app_data_dir().join("remote-jobs")
}

pub(crate) fn read_published_remote_transcript(path: &std::path::Path) -> Result<String, String> {
    remote::read_published_remote_transcript(path, &remote_jobs_directory())
        .map(|verified| verified.text)
}

pub(crate) fn authorize_published_remote_transcript(
    path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    remote::read_published_remote_transcript(path, &remote_jobs_directory())?;
    Ok(path.to_path_buf())
}
