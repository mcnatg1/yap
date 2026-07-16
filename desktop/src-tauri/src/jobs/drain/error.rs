use crate::server_connector::{
    batch::{ApiError, BatchClientError},
    BatchConnectionLease, ServerConnector,
};

const MAX_AUTOMATIC_REMOTE_ATTEMPTS: u64 = 6;

#[derive(Debug)]
pub(super) struct DrainStepError {
    pub(super) detail: String,
    pub(super) automatic_retry: bool,
    pub(super) code: &'static str,
    pub(super) user_message: &'static str,
}

impl DrainStepError {
    pub(super) fn permanent(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            automatic_retry: false,
            code: "REMOTE_STATE_INVALID",
            user_message: "The private-server job state is incompatible. Retry the recording to start a new server job.",
        }
    }

    pub(super) fn transient_state(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            automatic_retry: true,
            code: "REMOTE_REQUEST_RETRYING",
            user_message:
                "The private-server request did not complete. Yap will retry automatically.",
        }
    }

    pub(super) fn terminal_server(error: &ApiError) -> Self {
        Self {
            detail: format!(
                "server job failed with {} (request {}, retryable={})",
                error.code, error.request_id, error.retryable
            ),
            automatic_retry: false,
            code: "REMOTE_SERVER_FAILED",
            user_message: "The private server could not complete this recording. Retry it to start a new server job.",
        }
    }
}

impl std::fmt::Display for DrainStepError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl From<String> for DrainStepError {
    fn from(detail: String) -> Self {
        Self::permanent(detail)
    }
}

impl From<&str> for DrainStepError {
    fn from(detail: &str) -> Self {
        Self::permanent(detail)
    }
}

impl From<BatchClientError> for DrainStepError {
    fn from(error: BatchClientError) -> Self {
        if error.is_retryable() {
            Self::transient_state(error.to_string())
        } else {
            Self::permanent(error.to_string())
        }
    }
}

pub(super) type DrainResult<T> = Result<T, DrainStepError>;

pub(super) enum BatchCommitGuard<'a> {
    PersistedCleanup,
    #[cfg(test)]
    Unchecked,
    #[cfg(test)]
    StaleForTest,
    #[cfg(test)]
    StaleAfterForTest {
        remaining_successes: &'a std::sync::atomic::AtomicUsize,
    },
    Lease {
        connector: &'a ServerConnector,
        lease: &'a BatchConnectionLease,
    },
}

impl BatchCommitGuard<'_> {
    pub(super) fn commit<T>(&self, mutation: impl FnOnce() -> DrainResult<T>) -> DrainResult<T> {
        match self {
            Self::PersistedCleanup => mutation(),
            #[cfg(test)]
            Self::Unchecked => mutation(),
            #[cfg(test)]
            Self::StaleForTest => Err(DrainStepError::transient_state("test stale lease")),
            #[cfg(test)]
            Self::StaleAfterForTest {
                remaining_successes,
            } => {
                let remaining = remaining_successes.load(std::sync::atomic::Ordering::SeqCst);
                if remaining == 0 {
                    Err(DrainStepError::transient_state("test stale lease"))
                } else {
                    remaining_successes.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    mutation()
                }
            }
            Self::Lease { connector, lease } => connector
                .with_current_batch_lease(lease, mutation)
                .map_err(DrainStepError::transient_state)?,
        }
    }

    pub(super) fn ensure_current(&self) -> DrainResult<()> {
        self.commit(|| Ok(()))
    }
}

pub(super) fn remote_retry_plan(
    error: &DrainStepError,
    attempt_count: u64,
    updated_at_ms: u64,
) -> (Option<u64>, &'static str, &'static str) {
    let delay_seconds =
        [1_u64, 2, 4, 8, 15, 30][usize::try_from(attempt_count).unwrap_or(usize::MAX).min(5)];
    let retry_at_ms = (error.automatic_retry && attempt_count < MAX_AUTOMATIC_REMOTE_ATTEMPTS)
        .then(|| updated_at_ms.saturating_add(delay_seconds.saturating_mul(1_000)));
    if error.automatic_retry && retry_at_ms.is_none() {
        return (
            None,
            "REMOTE_RETRY_EXHAUSTED",
            "The private-server request did not recover after bounded retries. Retry the recording to start a new server job.",
        );
    }
    (retry_at_ms, error.code, error.user_message)
}
