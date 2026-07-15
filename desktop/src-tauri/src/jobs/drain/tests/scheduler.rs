use super::*;

#[test]
fn retention_drain_removes_pruned_private_spools_before_acknowledging_cleanup() {
    let dir = temp_dir("pruned-spool-cleanup");
    let remote_jobs_directory = dir.join("remote-jobs");
    let job_id = "job-0123456789abcdef01234567";
    let owned_spool = remote_jobs_directory.join(job_id);
    fs::create_dir_all(&owned_spool).unwrap();
    fs::write(owned_spool.join("private.pcm"), b"private bytes").unwrap();
    let ledger = JobLedger::open_in_memory().unwrap();
    {
        let connection = ledger.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO remote_spool_cleanup (job_id, queued_at_ms) VALUES (?1, 1)",
                [job_id],
            )
            .unwrap();
    }
    let resources = Arc::new(RecordingJobResources::from_storage(
        ledger,
        dir.join("recordings"),
        remote_jobs_directory,
    ));
    let drain = RemoteJobDrain::from_resources_for_test(
        Arc::clone(&resources),
        OwnerNamespace::local("i-pruned-spool-test").unwrap(),
    );

    assert!(drain.has_pending_work().unwrap());
    let mutation = resources.mutation().lock().unwrap();
    let (completed_tx, completed_rx) = std::sync::mpsc::channel();
    let retention = thread::spawn(move || {
        completed_tx.send(drain.enforce_retention(2)).unwrap();
    });
    assert!(completed_rx
        .recv_timeout(Duration::from_millis(50))
        .is_err());
    drop(mutation);
    assert!(completed_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .unwrap());
    retention.join().unwrap();
    assert!(!owned_spool.exists());
    assert!(resources
        .ledger()
        .list_pending_remote_spool_cleanup()
        .unwrap()
        .is_empty());

    drop(resources);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pending_owned_spool_cleanup_does_not_require_initialized_server_settings() {
    let dir = temp_dir("pending-spool-cleanup");
    let remote_jobs_directory = dir.join("remote-jobs");
    let job_id = "job-0123456789abcdef01234567";
    let owned_spool = remote_jobs_directory.join(job_id);
    fs::create_dir_all(&owned_spool).unwrap();
    fs::write(owned_spool.join("private.pcm"), b"private bytes").unwrap();
    let ledger = JobLedger::open_in_memory().unwrap();
    {
        let connection = ledger.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO remote_spool_cleanup (job_id, queued_at_ms) VALUES (?1, 1)",
                [job_id],
            )
            .unwrap();
    }
    let connector = ServerConnector::new();

    let cleaned = tauri::async_runtime::block_on(advance_persisted_cancellation_once(
        &ledger,
        &remote_jobs_directory,
        &connector,
        2,
    ))
    .unwrap();

    assert!(cleaned);
    assert!(!owned_spool.exists());
    assert!(ledger
        .list_pending_remote_spool_cleanup()
        .unwrap()
        .is_empty());

    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn automatic_remote_retries_are_typed_and_bounded() {
    let transient = DrainStepError::transient_state("request timed out");
    let permanent = DrainStepError::permanent("manifest conflicts with durable state");

    assert_eq!(
        remote_retry_plan(&transient, 0, 10_000),
        (
            Some(11_000),
            "REMOTE_REQUEST_RETRYING",
            "The private-server request did not complete. Yap will retry automatically.",
        )
    );
    assert_eq!(
            remote_retry_plan(&transient, 6, 10_000),
            (
                None,
                "REMOTE_RETRY_EXHAUSTED",
                "The private-server request did not recover after bounded retries. Retry the recording to start a new server job.",
            )
        );
    assert_eq!(
            remote_retry_plan(&permanent, 0, 10_000),
            (
                None,
                "REMOTE_STATE_INVALID",
                "The private-server job state is incompatible. Retry the recording to start a new server job.",
            )
        );
}

#[test]
fn terminal_server_diagnostics_do_not_copy_server_controlled_messages() {
    let private_message = "Private transcript and C:/private/audio.wav";
    let error = ApiError {
        code: "ASR_WORKER_FAILED".into(),
        message: private_message.into(),
        retryable: true,
        request_id: "job-abc123".into(),
    };

    let diagnostic = DrainStepError::terminal_server(&error);

    assert!(diagnostic.detail.contains("ASR_WORKER_FAILED"));
    assert!(diagnostic.detail.contains("job-abc123"));
    assert!(!diagnostic.detail.contains(private_message));
}
