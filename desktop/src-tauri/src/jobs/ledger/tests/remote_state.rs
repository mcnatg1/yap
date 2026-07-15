use super::*;

#[test]
fn prepared_remote_job_is_attached_atomically_and_survives_restart() {
    let dir = temp_dir("prepared-remote-restart");
    let database_path = dir.join("jobs.sqlite3");
    let source_path = dir.join("interview.wav");
    let manifest_path = dir.join("spool/job-remote/capture-manifest.json");
    let chunk_path = dir.join("spool/job-remote/track-1-0-9.pcm");
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(&source_path, b"RIFF-restart-fixture").unwrap();
    fs::write(&manifest_path, b"{}").unwrap();
    fs::write(&chunk_path, b"prepared audio bytes").unwrap();
    let create_request_json = r#"{"displayName":"interview.wav","route":"server_batch"}"#;

    {
        let ledger = JobLedger::open(&database_path).unwrap();
        let mut job = imported_job_at("job-remote", source_path.clone());
        job.status = RecordingJobStatus::QueuedServer;
        job.route = Some(RecordingRoute::ServerBatch);
        ledger.insert_job(&job).unwrap();
        ledger
            .transition("job-remote", RecordingJobStatus::Preprocessing, 101)
            .unwrap();
        let prepared = NewPreparedRemoteJob {
            create_request_json: create_request_json.into(),
            capture_manifest_path: manifest_path.clone(),
            capture_manifest_sha256:
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            chunks: vec![chunk_at(chunk_path.clone())],
        };

        let attached = ledger
            .attach_prepared_remote_job("job-remote", &prepared, 102)
            .unwrap();

        assert_eq!(attached.status, RecordingJobStatus::Uploading);
        assert!(ledger
            .attach_prepared_remote_job("job-remote", &prepared, 103)
            .is_err());
        assert_eq!(ledger.list_chunks("job-remote").unwrap().len(), 1);
    }

    let ledger = JobLedger::open(&database_path).unwrap();
    let recovered = ledger
        .get_prepared_remote_job("job-remote")
        .unwrap()
        .unwrap();
    assert_eq!(recovered.create_request_json, create_request_json);
    assert_eq!(recovered.capture_manifest_path, manifest_path);
    assert_eq!(recovered.server_job_id, None);
    assert_eq!(recovered.create_attempt_base_url, None);
    assert_eq!(
        ledger.get_job("job-remote").unwrap().unwrap().status,
        RecordingJobStatus::Uploading
    );
    assert_eq!(
        ledger.list_chunks("job-remote").unwrap()[0].artifact_path,
        chunk_path
    );
    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn remote_create_chunk_ack_and_commit_are_idempotent_and_restart_safe() {
    let dir = temp_dir("remote-progress-restart");
    let database_path = dir.join("jobs.sqlite3");
    let source_path = dir.join("interview.wav");
    let manifest_path = dir.join("spool/job-progress/capture-manifest.json");
    let chunk_path = dir.join("spool/job-progress/track-1-0-9.pcm");
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(&source_path, b"RIFF-restart-fixture").unwrap();
    fs::write(&manifest_path, b"{}").unwrap();
    fs::write(&chunk_path, b"prepared audio bytes").unwrap();

    {
        let ledger = JobLedger::open(&database_path).unwrap();
        let mut job = imported_job_at("job-progress", source_path);
        job.status = RecordingJobStatus::QueuedServer;
        job.route = Some(RecordingRoute::ServerBatch);
        ledger.insert_job(&job).unwrap();
        ledger
            .transition("job-progress", RecordingJobStatus::Preprocessing, 101)
            .unwrap();
        ledger
            .attach_prepared_remote_job(
                "job-progress",
                &NewPreparedRemoteJob {
                    create_request_json:
                        r#"{"displayName":"interview.wav","route":"server_batch"}"#.into(),
                    capture_manifest_path: manifest_path,
                    capture_manifest_sha256:
                        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                    chunks: vec![chunk_at(chunk_path)],
                },
                102,
            )
            .unwrap();

        ledger
            .begin_remote_create_attempt("job-progress", "http://127.0.0.1:18765", 103)
            .unwrap();
        assert_eq!(
            ledger
                .get_prepared_remote_job("job-progress")
                .unwrap()
                .unwrap()
                .create_attempt_base_url
                .as_deref(),
            Some("http://127.0.0.1:18765")
        );
        assert!(ledger
            .record_server_job_id(
                "job-progress",
                "job-server-1",
                "http://127.0.0.1:18766",
                104,
            )
            .is_err());
        ledger
            .record_server_job_id(
                "job-progress",
                "job-server-1",
                "http://127.0.0.1:18765",
                105,
            )
            .unwrap();
        ledger
            .record_server_job_id(
                "job-progress",
                "job-server-1",
                "http://127.0.0.1:18765",
                106,
            )
            .unwrap();
        assert!(ledger
            .record_server_job_id(
                "job-progress",
                "job-server-conflict",
                "http://127.0.0.1:18765",
                107,
            )
            .is_err());
        assert!(ledger
            .record_server_job_id(
                "job-progress",
                "job-server-1",
                "http://127.0.0.1:18766",
                107,
            )
            .is_err());
        assert_eq!(
            ledger
                .get_prepared_remote_job("job-progress")
                .unwrap()
                .unwrap()
                .create_attempt_base_url,
            None
        );
        assert!(ledger
            .mark_remote_job_committed("job-progress", 106)
            .is_err());

        ledger
            .acknowledge_remote_chunk(
                "job-progress",
                "microphone",
                0,
                9,
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                107,
            )
            .unwrap();
        ledger
            .acknowledge_remote_chunk(
                "job-progress",
                "microphone",
                0,
                9,
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                108,
            )
            .unwrap();
        ledger
            .mark_remote_job_committed("job-progress", 109)
            .unwrap();
    }

    let ledger = JobLedger::open(&database_path).unwrap();
    assert_eq!(
        ledger.get_job("job-progress").unwrap().unwrap().status,
        RecordingJobStatus::ServerProcessing
    );
    assert_eq!(
        ledger
            .get_prepared_remote_job("job-progress")
            .unwrap()
            .unwrap()
            .server_job_id
            .as_deref(),
        Some("job-server-1")
    );
    let chunks = ledger.list_chunks("job-progress").unwrap();
    assert_eq!(chunks[0].content_byte_length, 20);
    assert_eq!(chunks[0].upload_offset, chunks[0].content_byte_length);
    assert_eq!(chunks[0].acknowledged_at_ms, Some(107));
    let cancelled = ledger.request_cancellation("job-progress", 110).unwrap();
    assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
    let pending = ledger.list_pending_remote_cancellations().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].server_job_id.as_deref(), Some("job-server-1"));
    assert_eq!(
        pending[0].server_base_url.as_deref(),
        Some("http://127.0.0.1:18765")
    );
    ledger
        .acknowledge_server_cancellation("job-progress", "job-server-1", 111)
        .unwrap();
    assert!(ledger
        .list_pending_remote_cancellations()
        .unwrap()
        .is_empty());
    drop(ledger);
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn failed_remote_retry_detaches_the_old_binding_into_the_cancellation_outbox() {
    let dir = temp_dir("remote-retry-reset");
    let source_path = dir.join("source.wav");
    let manifest_path = dir.join("spool/job-retry/capture-manifest.json");
    let chunk_path = dir.join("spool/job-retry/track-1-0-9.pcm");
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(&source_path, b"RIFF-retry-fixture").unwrap();
    fs::write(&manifest_path, b"{}").unwrap();
    fs::write(&chunk_path, b"prepared audio bytes").unwrap();
    let ledger = JobLedger::open_in_memory().unwrap();
    let mut job = imported_job_at("job-retry", source_path);
    job.status = RecordingJobStatus::QueuedServer;
    job.route = Some(RecordingRoute::ServerBatch);
    ledger.insert_job(&job).unwrap();
    ledger
        .transition("job-retry", RecordingJobStatus::Preprocessing, 201)
        .unwrap();
    ledger
        .attach_prepared_remote_job(
            "job-retry",
            &NewPreparedRemoteJob {
                create_request_json: r#"{"displayName":"interview.wav","route":"server_batch"}"#
                    .into(),
                capture_manifest_path: manifest_path,
                capture_manifest_sha256:
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                chunks: vec![chunk_at(chunk_path)],
            },
            202,
        )
        .unwrap();
    ledger
        .begin_remote_create_attempt("job-retry", "http://127.0.0.1:18765", 203)
        .unwrap();
    ledger
        .record_server_job_id(
            "job-retry",
            "job-server-retry",
            "http://127.0.0.1:18765",
            203,
        )
        .unwrap();
    let failed = ledger
        .record_remote_error(
            "job-retry",
            "SERVER_CONTRACT_ERROR",
            "The private server returned incompatible job state.",
            None,
            204,
        )
        .unwrap();
    assert_eq!(failed.status, RecordingJobStatus::Failed);

    let retried = ledger
        .retry_to_queued_server("job-retry", 205, Some(604_800_205))
        .unwrap();
    assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
    assert_eq!(retried.error_code, None);
    assert_eq!(retried.capture_manifest_sha256, None);
    assert!(ledger
        .get_prepared_remote_job("job-retry")
        .unwrap()
        .is_none());
    assert!(ledger.list_chunks("job-retry").unwrap().is_empty());
    let cancellations = ledger.list_detached_remote_cancellations().unwrap();
    assert_eq!(cancellations.len(), 1);
    assert_eq!(cancellations[0].server_job_id, "job-server-retry");
    assert_eq!(cancellations[0].server_base_url, "http://127.0.0.1:18765");
    ledger
        .acknowledge_detached_remote_cancellation("http://127.0.0.1:18765", "job-server-retry")
        .unwrap();
    assert!(ledger
        .list_detached_remote_cancellations()
        .unwrap()
        .is_empty());
    fs::remove_dir_all(dir).unwrap();
}
