use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yap_desktop_lib::jobs::{
    JobLedger, NewRecordingJob, RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin,
    SourceOwnership,
};

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn file_backed_job_identity_status_and_attempts_survive_reopen() {
    let temp = TestDir::new("restart");
    let database = temp.path().join("jobs.sqlite3");
    let source = temp.path().join("restart.wav");
    fs::write(&source, b"RIFF-restart-proof").unwrap();

    {
        let ledger = JobLedger::open(&database).unwrap();
        ledger
            .insert_job(&accepted_job("restart-job", &source))
            .unwrap();
        ledger
            .accept_to_queued_server("restart-job", 101, 10_000)
            .unwrap();
        ledger
            .fail_source_validation("restart-job", "SERVER_UNAVAILABLE", 102)
            .unwrap();
        ledger
            .retry_to_queued_server("restart-job", 103, Some(20_000))
            .unwrap();
        ledger
            .fail_source_validation("restart-job", "SERVER_UNAVAILABLE", 104)
            .unwrap();
    }

    let reopened = JobLedger::open(&database).unwrap();
    let persisted = reopened.get_job("restart-job").unwrap().unwrap();

    assert_eq!(persisted.job_id, "restart-job");
    assert_eq!(persisted.status, RecordingJobStatus::Failed);
    assert_eq!(persisted.attempt_count, 1);
    assert_eq!(persisted.error_code.as_deref(), Some("SERVER_UNAVAILABLE"));
}

#[test]
fn terminal_history_stays_bounded_across_reopen() {
    const MAX_TERMINAL_HISTORY: usize = 500;
    let temp = TestDir::new("terminal-history");
    let database = temp.path().join("jobs.sqlite3");
    let source = temp.path().join("terminal.wav");
    fs::write(&source, b"RIFF-terminal-history-proof").unwrap();

    {
        let ledger = JobLedger::open(&database).unwrap();
        for index in 0..=MAX_TERMINAL_HISTORY {
            let job_id = format!("terminal-{index:04}");
            ledger.insert_job(&accepted_job(&job_id, &source)).unwrap();
            ledger
                .request_cancellation(&job_id, 1_000 + index as u64)
                .unwrap();
        }
    }

    let reopened = JobLedger::open(&database).unwrap();
    let rows = reopened.list_jobs().unwrap();
    assert_eq!(rows.len(), MAX_TERMINAL_HISTORY);
    assert!(rows
        .iter()
        .all(|row| row.status == RecordingJobStatus::Cancelled));
    assert!(reopened.get_job("terminal-0000").unwrap().is_none());
    assert!(reopened.get_job("terminal-0500").unwrap().is_some());
}

fn accepted_job(job_id: &str, source: &Path) -> NewRecordingJob {
    NewRecordingJob {
        job_id: job_id.to_owned(),
        session_mode: SessionMode::Meeting,
        session_origin: SessionOrigin::ImportedFile,
        source_path: Some(source.to_owned()),
        source_ownership: SourceOwnership::External,
        output_path: None,
        display_name: source
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .to_owned(),
        status: RecordingJobStatus::Accepted,
        route: Some(RecordingRoute::ServerBatch),
        attempt_count: 0,
        next_attempt_at_ms: None,
        cancellation_requested: false,
        capture_commit_path: None,
        capture_manifest_sha256: None,
        error_code: None,
        error_message: None,
        created_at_ms: 100,
        updated_at_ms: 100,
        expires_at_ms: Some(10_000),
    }
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yap-task7-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}
