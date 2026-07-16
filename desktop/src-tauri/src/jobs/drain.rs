mod contract;
mod error;
mod owner;
mod preparation;
mod processing;
mod recovery;
mod scheduler;
mod upload;

use error::{BatchCommitGuard, DrainResult, DrainStepError};
pub(crate) use owner::RemoteJobDrain;
pub(crate) use scheduler::start;

#[cfg(test)]
use contract::validate_result_revision;
#[cfg(test)]
use error::remote_retry_plan;
#[cfg(test)]
use preparation::{attach_prepared_remote_job_or_cleanup, prepare_next_queued_job};
#[cfg(test)]
use processing::{advance_processing_once, advance_processing_once_guarded};
#[cfg(test)]
use recovery::{advance_cancellation_once, advance_persisted_cancellation_once};
#[cfg(test)]
use upload::{advance_upload_once, advance_upload_once_guarded};

#[cfg(test)]
#[path = "drain/tests.rs"]
mod tests;
