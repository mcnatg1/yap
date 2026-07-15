mod error;
mod records;
mod status;
mod view;

pub use error::JobLedgerError;
pub use records::{
    DetachedRemoteCancellationRecord, JobChunkRecord, NewJobChunk, NewPreparedRemoteJob,
    NewRecordingJob, PreparedRemoteJobRecord, RecordingJobRecord,
};
pub use status::{RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin, SourceOwnership};
pub use view::RecordingJobView;

pub(crate) use status::{transition_policy, TransitionPolicy};

#[cfg(test)]
mod tests;
