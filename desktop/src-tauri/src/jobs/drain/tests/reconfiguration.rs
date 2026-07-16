use super::*;

#[test]
fn abandoned_create_attempt_is_recovered_and_cancelled_at_its_persisted_origin() {
    let root = temp_dir("abandoned-create-attempt");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-abandoned-create", source.clone()))
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
        .get_prepared_remote_job("job-abandoned-create")
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
        .begin_remote_create_attempt("job-abandoned-create", &base_url, 1_720_000_000_200)
        .unwrap();
    let boundary = ServerConnectorBoundary::new();
    boundary.configure(&ServerSettings::default());
    let connector = boundary.downgrade().upgrade().unwrap();

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

    let failed = ledger.get_job("job-abandoned-create").unwrap().unwrap();
    assert_eq!(failed.status, RecordingJobStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("REMOTE_ORIGIN_CHANGED"));
    assert!(ledger
        .get_prepared_remote_job("job-abandoned-create")
        .unwrap()
        .is_none());
    assert!(!remote_jobs.join("job-abandoned-create").exists());
    assert!(source.is_file(), "external source must never be deleted");
    let requests = observed.lock().unwrap();
    assert!(requests[0].starts_with("POST /v1/jobs HTTP/1.1"));
    assert!(requests[1].starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));

    drop(requests);
    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_origin_detaches_and_cancels_an_existing_server_binding() {
    let root = temp_dir("changed-origin-binding");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-changed-origin", source.clone()))
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
        .get_prepared_remote_job("job-changed-origin")
        .unwrap()
        .unwrap();
    let request =
        CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
    let server_job_id = "job-0123456789abcdef0123456789abcdef";
    let cancelled = serde_json::json!({
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
    let (old_origin, observed, server) = start_json_server(vec![(202, cancelled)]);
    ledger
        .begin_remote_create_attempt("job-changed-origin", &old_origin, 1_720_000_000_200)
        .unwrap();
    ledger
        .record_server_job_id(
            "job-changed-origin",
            server_job_id,
            &old_origin,
            1_720_000_000_201,
        )
        .unwrap();
    let boundary = ServerConnectorBoundary::new();
    boundary.configure(&ServerSettings {
        enabled: true,
        base_url: Some("http://127.0.0.1:9".into()),
        ..ServerSettings::default()
    });
    let connector = boundary.downgrade().upgrade().unwrap();

    tauri::async_runtime::block_on(async {
        assert!(advance_persisted_cancellation_once(
            &ledger,
            &remote_jobs,
            &connector,
            1_720_000_000_300,
        )
        .await
        .unwrap());
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

    let failed = ledger.get_job("job-changed-origin").unwrap().unwrap();
    assert_eq!(failed.status, RecordingJobStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("REMOTE_ORIGIN_CHANGED"));
    assert!(ledger
        .get_prepared_remote_job("job-changed-origin")
        .unwrap()
        .is_none());
    assert!(ledger
        .list_detached_remote_cancellations()
        .unwrap()
        .is_empty());
    assert!(!remote_jobs.join("job-changed-origin").exists());
    assert!(source.is_file(), "external source must never be deleted");
    assert!(observed.lock().unwrap()[0]
        .starts_with(&format!("DELETE /v1/jobs/{server_job_id} HTTP/1.1")));

    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}
