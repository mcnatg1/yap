use super::*;

#[test]
fn completed_server_result_is_published_before_the_ledger_becomes_complete() {
    let root = temp_dir("result");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-drain-result", source))
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
        .get_prepared_remote_job("job-drain-result")
        .unwrap()
        .unwrap();
    let request =
        CreateRecordingJobRequest::decode_persisted(&prepared.create_request_json).unwrap();
    let server_job_id = "job-0123456789abcdef0123456789abcdef";
    let projection = serde_json::json!({
        "jobId": server_job_id,
        "sessionId": request.metadata.session_id.as_str(),
        "displayName": request.display_name,
        "sessionMode": "meeting",
        "sessionOrigin": "imported_file",
        "status": "complete",
        "route": "server_batch",
        "captureManifest": request.capture_manifest,
        "createdAtUtc": "2026-07-14T21:00:00Z",
        "updatedAtUtc": "2026-07-14T21:00:02Z"
    });
    let result = serde_json::json!({
        "sessionId": request.metadata.session_id.as_str(),
        "revision": 1,
        "authority": "server_authoritative",
        "createdAtUtc": "2026-07-14T21:00:02Z",
        "captureManifestSha256": request.capture_manifest.sha256,
        "previousResultSha256": null,
        "status": "complete",
        "language": {
            "languageBcp47": "en-US",
            "confidence": null
        },
        "transcript": "Phase five is connected.",
        "alignedWords": [],
        "modelProvenance": [{
            "modelId": "CohereLabs/cohere-transcribe-03-2026",
            "revision": "b1eacc2686a3d08ceaae5f24a88b1d519620bc09",
            "calibrationRevision": "asr-not-applicable"
        }]
    });
    let valid_result: crate::server_connector::batch::TranscriptResultRevision =
        serde_json::from_value(result.clone()).unwrap();
    let mut empty_result = valid_result.clone();
    empty_result.transcript = " \n\t".into();
    assert!(validate_result_revision(&empty_result, &request).is_err());
    let mut offset_timestamp = valid_result;
    offset_timestamp.created_at_utc = "2026-07-14T16:00:02-05:00".into();
    assert!(validate_result_revision(&offset_timestamp, &request).is_err());
    let (base_url, observed, server) = start_json_server(vec![(200, projection), (200, result)]);
    ledger
        .begin_remote_create_attempt("job-drain-result", &base_url, 1_720_000_000_200)
        .unwrap();
    ledger
        .record_server_job_id(
            "job-drain-result",
            server_job_id,
            &base_url,
            1_720_000_000_200,
        )
        .unwrap();
    for chunk in &request.chunks {
        ledger
            .acknowledge_remote_chunk(
                "job-drain-result",
                &chunk.replay_key.track_id,
                chunk.replay_key.sequence_start,
                chunk.replay_key.sequence_end,
                &chunk.content_identity.sha256,
                1_720_000_000_300,
            )
            .unwrap();
    }
    ledger
        .mark_remote_job_committed("job-drain-result", 1_720_000_000_400)
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
        assert!(
            super::advance_processing_once(&ledger, &remote_jobs, &client, 1_720_000_000_500,)
                .await
                .unwrap()
        );
    });
    server.join().unwrap();

    let completed = ledger.get_job("job-drain-result").unwrap().unwrap();
    assert_eq!(completed.status, RecordingJobStatus::Complete);
    assert_eq!(completed.expires_at_ms, Some(1_722_592_000_000));
    let output = completed.output_path.unwrap();
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "Phase five is connected.\n"
    );
    let result_path = output.parent().unwrap().join("result.json");
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(result_path).unwrap()).unwrap();
    assert_eq!(
        persisted["captureManifestSha256"],
        request.capture_manifest.sha256
    );
    let requests = observed.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with(&format!("GET /v1/jobs/{server_job_id} HTTP/1.1")));
    assert!(requests[1].starts_with(&format!("GET /v1/jobs/{server_job_id}/result HTTP/1.1")));
    drop(requests);
    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}
