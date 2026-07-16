use super::*;

#[test]
fn queued_wav_is_preprocessed_into_durable_owned_replay_state() {
    let root = temp_dir("prepare");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let owned_live = root.join("live-recordings");
    let remote_jobs = root.join("remote-jobs");
    fs::create_dir_all(&owned_live).unwrap();
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let original = fs::read(&source).unwrap();
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&NewRecordingJob {
            job_id: "job-drain-prepare".into(),
            session_mode: SessionMode::Meeting,
            session_origin: SessionOrigin::ImportedFile,
            source_path: Some(source.clone()),
            source_ownership: SourceOwnership::External,
            output_path: None,
            display_name: "source.wav".into(),
            status: RecordingJobStatus::QueuedServer,
            route: Some(RecordingRoute::ServerBatch),
            attempt_count: 0,
            next_attempt_at_ms: None,
            cancellation_requested: false,
            capture_commit_path: None,
            capture_manifest_sha256: None,
            error_code: None,
            error_message: None,
            created_at_ms: 1_720_000_000_000,
            updated_at_ms: 1_720_000_000_000,
            expires_at_ms: Some(1_720_604_800_000),
        })
        .unwrap();
    let owner = OwnerNamespace::local("i-drain-test").unwrap();

    assert!(prepare_next_queued_job(
        &ledger,
        &owned_live,
        &remote_jobs,
        &owner,
        1_720_000_000_100,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .unwrap());

    let job = ledger.get_job("job-drain-prepare").unwrap().unwrap();
    assert_eq!(job.status, RecordingJobStatus::Uploading);
    let prepared = ledger
        .get_prepared_remote_job("job-drain-prepare")
        .unwrap()
        .unwrap();
    assert!(prepared.capture_manifest_path.is_file());
    assert_eq!(ledger.list_chunks("job-drain-prepare").unwrap().len(), 1);
    assert_eq!(fs::read(source).unwrap(), original);

    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_cancelled_preprocessing_race_removes_the_unattached_owned_spool() {
    let root = temp_dir("prepare-cancel-race");
    let database = root.join("jobs.sqlite3");
    let source = root.join("source.wav");
    let remote_jobs = root.join("remote-jobs");
    write_pcm_wav(&source, &vec![0_u8; 320]);
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&queued_job("job-prepare-cancel-race", source.clone()))
        .unwrap();
    ledger
        .transition(
            "job-prepare-cancel-race",
            RecordingJobStatus::Preprocessing,
            1_720_000_000_100,
        )
        .unwrap();
    let owner = OwnerNamespace::local("i-drain-test").unwrap();
    let mut source_file = File::open(&source).unwrap();
    let prepared = crate::jobs::remote::prepare_imported_pcm_wav(
        "job-prepare-cancel-race",
        "source.wav",
        &mut source_file,
        &remote_jobs,
        &owner,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .unwrap()
    .into_ledger_state()
    .unwrap();
    assert!(remote_jobs.join("job-prepare-cancel-race").is_dir());
    ledger
        .request_cancellation("job-prepare-cancel-race", 1_720_000_000_200)
        .unwrap();

    assert!(attach_prepared_remote_job_or_cleanup(
        &ledger,
        "job-prepare-cancel-race",
        &prepared,
        &remote_jobs,
        1_720_000_000_300,
    )
    .is_err());
    assert!(!remote_jobs.join("job-prepare-cancel-race").exists());
    assert!(source.is_file(), "external source must never be deleted");

    drop(ledger);
    fs::remove_dir_all(root).unwrap();
}
