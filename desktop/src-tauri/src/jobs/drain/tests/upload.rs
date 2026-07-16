use super::*;

#[test]
fn prepared_job_creates_uploads_and_commits_through_the_durable_contract() {
    let root = temp_dir("upload");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-drain-upload", source))
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
        .get_prepared_remote_job("job-drain-upload")
        .unwrap()
        .unwrap();
    let request =
        CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
    let create_idempotency_key = request.create_idempotency_key().unwrap();
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
    let chunk = &request.chunks[0];
    let responses = vec![
        (202, projection("accepted")),
        (
            201,
            serde_json::json!({
                "replayKey": chunk.replay_key,
                "contentIdentity": chunk.content_identity,
                "disposition": "accepted",
                "acceptedAtUtc": "2026-07-14T21:00:01Z"
            }),
        ),
        (200, projection("uploading")),
        (202, projection("server_processing")),
    ];
    let (base_url, observed, server) = start_json_server(responses);
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
        assert!(
            advance_upload_once(&ledger, &remote_jobs, &client, 1_720_000_000_200,)
                .await
                .unwrap()
        );
        assert!(
            advance_upload_once(&ledger, &remote_jobs, &client, 1_720_000_000_300,)
                .await
                .unwrap()
        );
        assert!(
            advance_upload_once(&ledger, &remote_jobs, &client, 1_720_000_000_400,)
                .await
                .unwrap()
        );
    });
    server.join().unwrap();

    assert_eq!(
        ledger.get_job("job-drain-upload").unwrap().unwrap().status,
        RecordingJobStatus::ServerProcessing
    );
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].starts_with("POST /v1/jobs HTTP/1.1"));
    assert!(requests[0]
        .to_ascii_lowercase()
        .contains(&format!("idempotency-key: {create_idempotency_key}")));
    assert!(requests[1].starts_with(&format!(
        "PUT /v1/jobs/{server_job_id}/chunks/track-1/0-159 HTTP/1.1"
    )));
    assert!(requests[2].starts_with(&format!("GET /v1/jobs/{server_job_id} HTTP/1.1")));
    assert!(requests[3].starts_with(&format!("POST /v1/jobs/{server_job_id}/commit HTTP/1.1")));
    drop(requests);
    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_lease_is_rejected_before_create_upload_or_processing_dispatch() {
    let root = temp_dir("stale-pre-dispatch");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-stale-pre-dispatch", source))
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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let client = BatchApiClient::new(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(100))
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap(),
        &base_url,
    )
    .unwrap();

    tauri::async_runtime::block_on(async {
        let create_error = advance_upload_once_guarded(
            &ledger,
            &remote_jobs,
            &client,
            1_720_000_000_200,
            &BatchCommitGuard::StaleForTest,
        )
        .await
        .unwrap_err();
        assert_eq!(create_error.detail, "test stale lease");

        ledger
            .begin_remote_create_attempt("job-stale-pre-dispatch", &base_url, 1_720_000_000_300)
            .unwrap();
        ledger
            .record_server_job_id(
                "job-stale-pre-dispatch",
                "job-0123456789abcdef0123456789abcdef",
                &base_url,
                1_720_000_000_300,
            )
            .unwrap();
        let upload_error = advance_upload_once_guarded(
            &ledger,
            &remote_jobs,
            &client,
            1_720_000_000_400,
            &BatchCommitGuard::StaleForTest,
        )
        .await
        .unwrap_err();
        assert_eq!(upload_error.detail, "test stale lease");
        assert!(ledger
            .list_chunks("job-stale-pre-dispatch")
            .unwrap()
            .iter()
            .all(|chunk| chunk.acknowledged_at_ms.is_none()));

        let chunk = ledger
            .list_chunks("job-stale-pre-dispatch")
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        ledger
            .acknowledge_remote_chunk(
                "job-stale-pre-dispatch",
                &chunk.track_id,
                chunk.sequence_start,
                chunk.sequence_end,
                &chunk.content_sha256,
                1_720_000_000_500,
            )
            .unwrap();
        ledger
            .mark_remote_job_committed("job-stale-pre-dispatch", 1_720_000_000_600)
            .unwrap();
        let processing_error = advance_processing_once_guarded(
            &ledger,
            &remote_jobs,
            &client,
            1_720_000_000_700,
            &BatchCommitGuard::StaleForTest,
        )
        .await
        .unwrap_err();
        assert_eq!(processing_error.detail, "test stale lease");
    });

    let prepared = ledger
        .get_prepared_remote_job("job-stale-pre-dispatch")
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared.server_job_id.as_deref(),
        Some("job-0123456789abcdef0123456789abcdef")
    );
    assert_eq!(
        ledger
            .get_job("job-stale-pre-dispatch")
            .unwrap()
            .unwrap()
            .status,
        RecordingJobStatus::ServerProcessing
    );

    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn create_response_binding_is_durable_when_the_connector_changes_in_flight() {
    let root = temp_dir("stale-create-response");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-stale-create-response", source))
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
        .get_prepared_remote_job("job-stale-create-response")
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
        "status": "accepted",
        "route": "server_batch",
        "captureManifest": request.capture_manifest,
        "createdAtUtc": "2026-07-14T21:00:00Z",
        "updatedAtUtc": "2026-07-14T21:00:01Z"
    });
    let (base_url, observed, server) = start_json_server(vec![(202, response)]);
    let client = BatchApiClient::new(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        &base_url,
    )
    .unwrap();
    let remaining_successes = std::sync::atomic::AtomicUsize::new(2);

    let error = tauri::async_runtime::block_on(advance_upload_once_guarded(
        &ledger,
        &remote_jobs,
        &client,
        1_720_000_000_200,
        &BatchCommitGuard::StaleAfterForTest {
            remaining_successes: &remaining_successes,
        },
    ))
    .unwrap_err();
    server.join().unwrap();

    assert_eq!(error.detail, "test stale lease");
    let durable = ledger
        .get_prepared_remote_job("job-stale-create-response")
        .unwrap()
        .unwrap();
    assert_eq!(durable.server_job_id.as_deref(), Some(server_job_id));
    assert_eq!(durable.server_base_url.as_deref(), Some(base_url.as_str()));
    assert_eq!(durable.create_attempt_base_url, None);
    assert_eq!(observed.lock().unwrap().len(), 1);

    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}
