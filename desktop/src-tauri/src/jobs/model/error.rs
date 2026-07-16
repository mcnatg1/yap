use super::status::RecordingJobStatus;
use std::{fmt, path::PathBuf};

#[derive(Debug)]
pub enum JobLedgerError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    OwnedSpoolCleanup(String),
    CorruptValue {
        field: &'static str,
        value: String,
    },
    OutOfRange {
        field: &'static str,
        value: u64,
    },
    InvalidPath {
        field: &'static str,
        path: PathBuf,
    },
    InvalidRecord(&'static str),
    PragmaNotApplied {
        pragma: &'static str,
        requested: &'static str,
        actual: String,
    },
    UnsupportedSchema(i64),
    NotFound(String),
    InvalidTransition {
        from: RecordingJobStatus,
        to: RecordingJobStatus,
    },
    RetryRequired,
    CancellationRequired,
    DismissRequired,
    LockPoisoned,
}

impl fmt::Display for JobLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "job ledger database error: {error}"),
            Self::Io(error) => write!(formatter, "job ledger filesystem error: {error}"),
            Self::OwnedSpoolCleanup(error) => {
                write!(formatter, "owned remote spool cleanup failed: {error}")
            }
            Self::CorruptValue { field, value } => {
                write!(formatter, "job ledger has invalid {field} value {value:?}")
            }
            Self::OutOfRange { field, value } => {
                write!(
                    formatter,
                    "{field} value {value} exceeds SQLite's signed integer range"
                )
            }
            Self::InvalidPath { field, path } => {
                write!(
                    formatter,
                    "{field} must be an absolute UTF-8 path: {}",
                    path.display()
                )
            }
            Self::InvalidRecord(message) => formatter.write_str(message),
            Self::PragmaNotApplied {
                pragma,
                requested,
                actual,
            } => write!(
                formatter,
                "job ledger requested {requested} for {pragma}, but SQLite applied {actual}"
            ),
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "job ledger uses unsupported schema version {version}"
                )
            }
            Self::NotFound(job_id) => write!(formatter, "recording job {job_id:?} was not found"),
            Self::InvalidTransition { from, to } => {
                write!(
                    formatter,
                    "recording job cannot transition from {from:?} to {to:?}"
                )
            }
            Self::RetryRequired => {
                formatter.write_str("retry transitions must use JobLedger::retry")
            }
            Self::CancellationRequired => formatter
                .write_str("cancellation transitions must use JobLedger::request_cancellation"),
            Self::DismissRequired => {
                formatter.write_str("dismiss transitions must use JobLedger::dismiss_failed")
            }
            Self::LockPoisoned => formatter.write_str("job ledger connection lock is poisoned"),
        }
    }
}

impl std::error::Error for JobLedgerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for JobLedgerError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for JobLedgerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
