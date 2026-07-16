use super::*;

#[test]
fn cancellation_updates_metadata_but_never_deletes_an_external_source() {
    let dir = temp_dir("external-cancel");
    let source = dir.join("user-owned.wav");
    fs::write(&source, b"RIFF-user-owned").unwrap();
    let ledger = JobLedger::open_in_memory().unwrap();
    ledger
        .insert_job(&imported_job_at("cancel-job", source.clone()))
        .unwrap();

    let cancelled = ledger.request_cancellation("cancel-job", 300).unwrap();
    assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
    assert!(cancelled.cancellation_requested);
    assert!(source.exists());
    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_dismissal_uses_its_central_policy_and_preserves_failure_provenance() {
    let dir = temp_dir("external-dismiss");
    let source = dir.join("user-owned-failed.wav");
    fs::write(&source, b"RIFF-user-owned-failed").unwrap();
    let ledger = JobLedger::open_in_memory().unwrap();
    let mut failed = imported_job_at("dismiss-job", source.clone());
    failed.status = RecordingJobStatus::Failed;
    failed.error_code = Some("PLAYBACK_AUTHORITY_FAILED".into());
    failed.error_message = Some("playback authority could not be established".into());
    ledger.insert_job(&failed).unwrap();

    assert!(matches!(
        ledger.transition("dismiss-job", RecordingJobStatus::Cancelled, 300),
        Err(JobLedgerError::DismissRequired)
    ));
    assert!(matches!(
        ledger.request_cancellation("dismiss-job", 301),
        Err(JobLedgerError::InvalidTransition { .. })
    ));
    let dismissed = ledger.dismiss_failed("dismiss-job", 302).unwrap();

    assert_eq!(dismissed.status, RecordingJobStatus::Cancelled);
    assert!(dismissed.cancellation_requested);
    assert_eq!(dismissed.source_path.as_deref(), Some(source.as_path()));
    assert_eq!(
        dismissed.error_code.as_deref(),
        Some("PLAYBACK_AUTHORITY_FAILED")
    );
    assert_eq!(
        dismissed.error_message.as_deref(),
        Some("playback authority could not be established")
    );
    assert!(source.exists());
    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_history_is_bounded_without_pruning_recoverable_or_current_jobs() {
    let ledger = JobLedger::open_in_memory().unwrap();
    ledger.insert_job(&imported_job("active-survivor")).unwrap();
    let mut failed = imported_job("failed-survivor");
    failed.status = RecordingJobStatus::Failed;
    ledger.insert_job(&failed).unwrap();

    for index in 0..MAX_TERMINAL_JOB_HISTORY {
        let id = format!("terminal-{index:04}");
        let mut job = imported_job(&id);
        if index == 0 {
            job.route = Some(RecordingRoute::ServerBatch);
        }
        if index == 0 {
            ledger
                .insert_job_with_chunks(&job, &[chunk_at(std::env::temp_dir().join("old.flac"))])
                .unwrap();
        } else {
            ledger.insert_job(&job).unwrap();
        }
        ledger
            .request_cancellation(&id, 1_000 + index as u64)
            .unwrap();
    }

    ledger
        .insert_job(&imported_job("protected-current"))
        .unwrap();
    ledger.request_cancellation("protected-current", 1).unwrap();

    let connection = ledger.connection.lock().unwrap();
    let terminal_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM recording_jobs WHERE status IN ('complete', 'partial', 'cancelled')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let pruned_chunk_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM job_chunks WHERE job_id = 'terminal-0000'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);

    assert_eq!(terminal_count, MAX_TERMINAL_JOB_HISTORY as i64);
    assert!(ledger.get_job("terminal-0000").unwrap().is_none());
    assert!(ledger.get_job("protected-current").unwrap().is_some());
    assert!(ledger.get_job("active-survivor").unwrap().is_some());
    assert_eq!(
        ledger.get_job("failed-survivor").unwrap().unwrap().status,
        RecordingJobStatus::Failed
    );
    assert_eq!(pruned_chunk_count, 0);
    assert_eq!(
        ledger.list_pending_remote_spool_cleanup().unwrap(),
        ["terminal-0000"]
    );
    assert!(ledger
        .acknowledge_remote_spool_cleanup("terminal-0000")
        .unwrap());
    assert!(ledger
        .list_pending_remote_spool_cleanup()
        .unwrap()
        .is_empty());
}

#[test]
fn terminal_pruning_preserves_a_cancelled_create_attempt_until_remote_cleanup() {
    let dir = temp_dir("cancelled-create-prune");
    let source = dir.join("source.wav");
    let manifest = dir.join("remote/job-create/capture-manifest.json");
    let chunk = dir.join("remote/job-create/chunk.pcm");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&source, b"RIFF-source").unwrap();
    fs::write(&manifest, b"{}").unwrap();
    fs::write(&chunk, b"private audio").unwrap();
    let ledger = JobLedger::open_in_memory().unwrap();
    let mut job = imported_job_at("job-create", source);
    job.status = RecordingJobStatus::QueuedServer;
    job.route = Some(RecordingRoute::ServerBatch);
    ledger.insert_job(&job).unwrap();
    ledger
        .transition("job-create", RecordingJobStatus::Preprocessing, 2)
        .unwrap();
    ledger
        .attach_prepared_remote_job(
            "job-create",
            &NewPreparedRemoteJob {
                create_request_json: "{}".into(),
                capture_manifest_path: manifest,
                capture_manifest_sha256: "a".repeat(64),
                chunks: vec![chunk_at(chunk)],
            },
            3,
        )
        .unwrap();
    ledger
        .begin_remote_create_attempt("job-create", "http://127.0.0.1:18765", 4)
        .unwrap();
    ledger.request_cancellation("job-create", 5).unwrap();

    for index in 0..MAX_TERMINAL_JOB_HISTORY {
        let id = format!("later-terminal-{index:04}");
        ledger.insert_job(&imported_job(&id)).unwrap();
        ledger
            .request_cancellation(&id, 1_000 + index as u64)
            .unwrap();
    }

    assert!(ledger.get_job("job-create").unwrap().is_some());
    let pending = ledger
        .get_prepared_remote_job("job-create")
        .unwrap()
        .expect("cancelled create attempt remains recoverable");
    assert_eq!(
        pending.create_attempt_base_url.as_deref(),
        Some("http://127.0.0.1:18765")
    );
    assert!(ledger
        .list_pending_remote_spool_cleanup()
        .unwrap()
        .is_empty());

    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reopening_legacy_database_prunes_preexisting_terminal_overflow_and_chunks() {
    let dir = temp_dir("terminal-reopen-prune");
    let path = dir.join("jobs.sqlite3");
    {
        let ledger = JobLedger::open(&path).unwrap();
        ledger.insert_job(&imported_job("active-survivor")).unwrap();
        let mut failed = imported_job("failed-survivor");
        failed.status = RecordingJobStatus::Failed;
        ledger.insert_job(&failed).unwrap();

        ledger
            .insert_job_with_chunks(
                &imported_job("terminal-0000"),
                &[chunk_at(dir.join("old-terminal.flac"))],
            )
            .unwrap();
        let overflow = (1..=MAX_TERMINAL_JOB_HISTORY)
            .map(|index| imported_job(&format!("terminal-{index:04}")))
            .collect::<Vec<_>>();
        ledger.insert_jobs(&overflow).unwrap();

        let mut connection = ledger.connection.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "UPDATE recording_jobs SET route = 'server_batch' WHERE job_id = 'terminal-0000'",
                [],
            )
            .unwrap();
        for index in 0..=MAX_TERMINAL_JOB_HISTORY {
            transaction
                .execute(
                    "UPDATE recording_jobs SET status = 'cancelled', updated_at_ms = ?1 WHERE job_id = ?2",
                    params![1_000 + index as i64, format!("terminal-{index:04}")],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }

    let reopened = JobLedger::open(&path).unwrap();
    let connection = reopened.connection.lock().unwrap();
    let terminal_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM recording_jobs WHERE status IN ('complete', 'partial', 'cancelled')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let pruned_chunk_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM job_chunks WHERE job_id = 'terminal-0000'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    drop(connection);

    assert_eq!(terminal_count, MAX_TERMINAL_JOB_HISTORY as i64);
    assert!(reopened.get_job("terminal-0000").unwrap().is_none());
    assert!(reopened.get_job("terminal-0500").unwrap().is_some());
    assert!(reopened.get_job("active-survivor").unwrap().is_some());
    assert_eq!(
        reopened.get_job("failed-survivor").unwrap().unwrap().status,
        RecordingJobStatus::Failed
    );
    assert_eq!(pruned_chunk_count, 0);
    assert_eq!(
        reopened.list_pending_remote_spool_cleanup().unwrap(),
        ["terminal-0000"]
    );
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}
