use super::*;

#[test]
fn concurrent_callers_can_read_the_mutex_owned_connection() {
    let ledger = Arc::new(JobLedger::open_in_memory().unwrap());
    ledger.insert_job(&imported_job("concurrent-job")).unwrap();
    let barrier = Arc::new(Barrier::new(9));
    let readers: Vec<_> = (0..8)
        .map(|_| {
            let ledger = Arc::clone(&ledger);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for _ in 0..50 {
                    assert_eq!(
                        ledger.get_job("concurrent-job").unwrap().unwrap().job_id,
                        "concurrent-job"
                    );
                }
            })
        })
        .collect();
    barrier.wait();
    for reader in readers {
        reader.join().unwrap();
    }
}

#[test]
fn job_and_chunk_insert_rolls_back_as_one_transaction() {
    let ledger = JobLedger::open_in_memory().unwrap();
    let chunk = chunk_at(std::env::temp_dir().join("duplicate-chunk.flac"));
    let error = ledger
        .insert_job_with_chunks(&imported_job("rollback-job"), &[chunk.clone(), chunk])
        .unwrap_err();
    assert!(matches!(error, JobLedgerError::Sqlite(_)));
    assert!(ledger.get_job("rollback-job").unwrap().is_none());
}

#[test]
fn multi_job_insert_rolls_back_every_row_when_one_insert_fails() {
    let ledger = JobLedger::open_in_memory().unwrap();
    let first = imported_job("duplicate-job");
    let second = imported_job("duplicate-job");

    assert!(ledger.insert_jobs(&[first, second]).is_err());
    assert!(ledger.list_jobs().unwrap().is_empty());
}

#[test]
fn every_unsigned_sql_value_is_range_checked_without_partial_writes() {
    type JobMutation = fn(&mut NewRecordingJob);
    type ChunkMutation = fn(&mut NewJobChunk);

    let ledger = JobLedger::open_in_memory().unwrap();
    let job_cases: [(&str, JobMutation); 5] = [
        ("attempt", |job: &mut NewRecordingJob| {
            job.attempt_count = u64::MAX
        }),
        ("next", |job: &mut NewRecordingJob| {
            job.next_attempt_at_ms = Some(u64::MAX)
        }),
        ("created", |job: &mut NewRecordingJob| {
            job.created_at_ms = u64::MAX
        }),
        ("updated", |job: &mut NewRecordingJob| {
            job.updated_at_ms = u64::MAX
        }),
        ("expires", |job: &mut NewRecordingJob| {
            job.expires_at_ms = Some(u64::MAX)
        }),
    ];
    for (id, mutate) in job_cases {
        let mut job = imported_job(id);
        mutate(&mut job);
        assert!(matches!(
            ledger.insert_job(&job),
            Err(JobLedgerError::OutOfRange { .. })
        ));
        assert!(ledger.get_job(id).unwrap().is_none());
    }

    let chunk_cases: [(&str, ChunkMutation); 5] = [
        ("seq-start", |chunk: &mut NewJobChunk| {
            chunk.sequence_start = u64::MAX
        }),
        ("seq-end", |chunk: &mut NewJobChunk| {
            chunk.sequence_end = u64::MAX
        }),
        ("offset", |chunk: &mut NewJobChunk| {
            chunk.upload_offset = u64::MAX
        }),
        ("byte-length", |chunk: &mut NewJobChunk| {
            chunk.content_byte_length = u64::MAX
        }),
        ("ack-at", |chunk: &mut NewJobChunk| {
            chunk.acknowledged_at_ms = Some(u64::MAX)
        }),
    ];
    for (id, mutate) in chunk_cases {
        let mut chunk = chunk_at(std::env::temp_dir().join(format!("{id}.flac")));
        mutate(&mut chunk);
        assert!(matches!(
            ledger.insert_job_with_chunks(&imported_job(id), &[chunk]),
            Err(JobLedgerError::OutOfRange { .. })
        ));
        assert!(ledger.get_job(id).unwrap().is_none());
    }
}

#[test]
fn retry_is_transactional_and_never_skips_preflight() {
    let ledger = JobLedger::open_in_memory().unwrap();
    let mut failed = imported_job("retry-job");
    failed.status = RecordingJobStatus::Failed;
    failed.attempt_count = 3;
    ledger.insert_job(&failed).unwrap();

    assert_eq!(
        transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Uploading),
        TransitionPolicy::Forbidden
    );
    assert!(matches!(
        ledger.transition("retry-job", RecordingJobStatus::Uploading, 200),
        Err(JobLedgerError::InvalidTransition { .. })
    ));
    let retried = ledger.retry("retry-job", 201).unwrap();
    assert_eq!(retried.status, RecordingJobStatus::Preflighting);
    assert_eq!(retried.attempt_count, 4);
    assert_eq!(retried.updated_at_ms, 201);
}

#[test]
fn retry_rejects_max_counter_before_waiting_for_a_writer_transaction() {
    let dir = temp_dir("retry-max-before-transaction");
    let path = dir.join("jobs.sqlite3");
    let ledger = Arc::new(JobLedger::open(&path).unwrap());
    let mut failed = imported_job("retry-max");
    failed.status = RecordingJobStatus::Failed;
    failed.attempt_count = i64::MAX as u64;
    ledger.insert_job(&failed).unwrap();

    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let retrying_ledger = Arc::clone(&ledger);
    let retry = thread::spawn(move || {
        result_tx
            .send(retrying_ledger.retry("retry-max", 202))
            .unwrap();
    });

    let early_result = result_rx.recv_timeout(std::time::Duration::from_millis(200));
    let was_early = early_result.is_ok();
    writer.execute_batch("ROLLBACK").unwrap();
    let result = match early_result {
        Ok(result) => result,
        Err(_) => result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
    };
    retry.join().unwrap();
    assert!(
        was_early,
        "retry opened a writer transaction before rejecting i64::MAX"
    );
    assert!(matches!(
        result,
        Err(JobLedgerError::OutOfRange {
            field: "attempt_count",
            value,
        }) if value == i64::MAX as u64 + 1
    ));

    drop(writer);
    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_retry_connections_increment_once_without_a_stale_overwrite() {
    let dir = temp_dir("concurrent-retry");
    let path = dir.join("jobs.sqlite3");
    let first = JobLedger::open(&path).unwrap();
    let mut failed = imported_job("concurrent-retry");
    failed.status = RecordingJobStatus::Failed;
    failed.attempt_count = 7;
    first.insert_job(&failed).unwrap();
    let second = JobLedger::open(&path).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let retries: Vec<_> = [first, second]
        .into_iter()
        .map(|ledger| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                ledger.retry("concurrent-retry", 203)
            })
        })
        .collect();

    barrier.wait();
    let results: Vec<_> = retries
        .into_iter()
        .map(|retry| retry.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(JobLedgerError::InvalidTransition { .. })))
            .count(),
        1
    );

    let observer = JobLedger::open(&path).unwrap();
    let record = observer.get_job("concurrent-retry").unwrap().unwrap();
    assert_eq!(record.status, RecordingJobStatus::Preflighting);
    assert_eq!(record.attempt_count, 8);
    drop(observer);
    fs::remove_dir_all(dir).unwrap();
}
