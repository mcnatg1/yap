use super::*;

#[test]
fn terminal_conflict_does_not_acknowledge_detached_cancellation() {
    let root = temp_dir("detached-cancel");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-detached-cancel", source))
        .unwrap();
    let owner = OwnerNamespace::local("i-drain-test").unwrap();
    prepare_next_queued_job(
        &ledger,
        &owned_live,
        &remote_jobs,
        &owner,
        1_720_000_000_100,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .unwrap();
    let server_job_id = "job-0123456789abcdef0123456789abcdef";
    let (base_url, observed, server) = start_json_server(vec![(
        409,
        serde_json::json!({
            "code": "JOB_TERMINAL",
            "message": "The server job is already terminal.",
            "retryable": false,
            "requestId": "req-detached-cancel"
        }),
    )]);
    ledger
        .begin_remote_create_attempt("job-detached-cancel", &base_url, 1_720_000_000_200)
        .unwrap();
    ledger
        .record_server_job_id(
            "job-detached-cancel",
            server_job_id,
            &base_url,
            1_720_000_000_200,
        )
        .unwrap();
    ledger
        .record_remote_error(
            "job-detached-cancel",
            "REMOTE_RETRY_EXHAUSTED",
            "The private server request did not recover.",
            None,
            1_720_000_000_300,
        )
        .unwrap();
    ledger
        .retry_to_queued_server(
            "job-detached-cancel",
            1_720_000_000_400,
            Some(1_720_604_800_400),
        )
        .unwrap();
    let client = BatchApiClient::new(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        &base_url,
    )
    .unwrap();

    tauri::async_runtime::block_on(async {
        let error = advance_cancellation_once(&ledger, &remote_jobs, &client, 1_720_000_000_500)
            .await
            .unwrap_err();
        assert_eq!(error.detail, "JOB_TERMINAL (HTTP 409)");
        assert!(!error.automatic_retry);
    });
    server.join().unwrap();

    assert_eq!(
        ledger.list_detached_remote_cancellations().unwrap().len(),
        1
    );
    assert!(observed.lock().unwrap()[0]
        .starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));
    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn persisted_origin_cancellation_does_not_require_a_current_connector_lease() {
    let root = temp_dir("current-cancel");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-current-cancel", source.clone()))
        .unwrap();
    let owner = OwnerNamespace::local("i-drain-test").unwrap();
    prepare_next_queued_job(
        &ledger,
        &owned_live,
        &remote_jobs,
        &owner,
        1_720_000_000_100,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .unwrap();
    let prepared = ledger
        .get_prepared_remote_job("job-current-cancel")
        .unwrap()
        .unwrap();
    let request =
        CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
    let server_job_id = "job-0123456789abcdef0123456789abcdef";
    let response = serde_json::json!({
        "jobId": server_job_id,
        "sessionId": request.metadata.session_id.as_str(),
        "displayName": request.display_name,
        "sessionMode": "meeting",
        "sessionOrigin": "imported_file",
        "status": "cancelled",
        "route": "server_batch",
        "captureManifest": request.capture_manifest,
        "createdAtUtc": "2026-07-14T21:00:00Z",
        "updatedAtUtc": "2026-07-14T21:00:01Z"
    });
    let (base_url, observed, server) = start_json_server(vec![(202, response)]);
    ledger
        .begin_remote_create_attempt("job-current-cancel", &base_url, 1_720_000_000_200)
        .unwrap();
    ledger
        .record_server_job_id(
            "job-current-cancel",
            server_job_id,
            &base_url,
            1_720_000_000_200,
        )
        .unwrap();
    ledger
        .request_cancellation("job-current-cancel", 1_720_000_000_300)
        .unwrap();
    let connector = ServerConnector::new();

    tauri::async_runtime::block_on(async {
        assert!(advance_persisted_cancellation_once(
            &ledger,
            &remote_jobs,
            &connector,
            1_720_000_000_400,
        )
        .await
        .unwrap());
    });
    server.join().unwrap();

    assert!(!remote_jobs.join("job-current-cancel").exists());
    assert!(source.is_file(), "external source must never be deleted");
    let acknowledged = ledger
        .get_prepared_remote_job("job-current-cancel")
        .unwrap()
        .unwrap();
    assert_eq!(
        acknowledged.server_cancellation_acknowledged_at_ms,
        Some(1_720_000_000_400)
    );
    assert!(observed.lock().unwrap()[0]
        .starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));
    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelled_inflight_create_is_recovered_before_its_local_tombstone_is_acknowledged() {
    let root = temp_dir("cancelled-create-attempt");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-cancelled-create", source.clone()))
        .unwrap();
    let owner = OwnerNamespace::local("i-drain-test").unwrap();
    prepare_next_queued_job(
        &ledger,
        &owned_live,
        &remote_jobs,
        &owner,
        1_720_000_000_100,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .unwrap();
    let prepared = ledger
        .get_prepared_remote_job("job-cancelled-create")
        .unwrap()
        .unwrap();
    let request =
        CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
    let server_job_id = "job-0123456789abcdef0123456789abcdef";
    let projection = |status: &str| {
        serde_json::json!({
            "jobId": server_job_id,
            "sessionId": request.metadata.session_id.as_str(),
            "displayName": request.display_name,
            "sessionMode": "meeting",
            "sessionOrigin": "imported_file",
            "status": status,
            "route": "server_batch",
            "captureManifest": request.capture_manifest,
            "createdAtUtc": "2026-07-14T21:00:00Z",
            "updatedAtUtc": "2026-07-14T21:00:01Z"
        })
    };
    let (base_url, observed, server) = start_json_server(vec![
        (202, projection("accepted")),
        (202, projection("cancelled")),
    ]);
    ledger
        .begin_remote_create_attempt("job-cancelled-create", &base_url, 1_720_000_000_200)
        .unwrap();
    ledger
        .request_cancellation("job-cancelled-create", 1_720_000_000_201)
        .unwrap();
    let pending_probe_resources = Arc::new(RecordingJobResources::from_storage(
        JobLedger::open(&database).unwrap(),
        owned_live.clone(),
        remote_jobs.clone(),
    ));
    let pending_probe = RemoteJobDrain::from_resources_for_test(
        pending_probe_resources,
        OwnerNamespace::local("i-pending-probe").unwrap(),
    );
    assert!(pending_probe.has_pending_work().unwrap());
    drop(pending_probe);
    let connector = ServerConnector::new();

    tauri::async_runtime::block_on(async {
        assert!(advance_persisted_cancellation_once(
            &ledger,
            &remote_jobs,
            &connector,
            1_720_000_000_300,
        )
        .await
        .unwrap());
    });
    server.join().unwrap();

    let acknowledged = ledger
        .get_prepared_remote_job("job-cancelled-create")
        .unwrap()
        .unwrap();
    assert_eq!(acknowledged.server_job_id.as_deref(), Some(server_job_id));
    assert_eq!(
        acknowledged.server_cancellation_acknowledged_at_ms,
        Some(1_720_000_000_300)
    );
    assert!(!remote_jobs.join("job-cancelled-create").exists());
    assert!(source.is_file(), "external source must never be deleted");
    let requests = observed.lock().unwrap();
    assert!(requests[0].starts_with("POST /v1/jobs HTTP/1.1"));
    assert!(requests[1].starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));

    drop(requests);
    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}
