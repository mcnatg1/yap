use super::*;

#[test]
fn mutation_adapter_notifies_even_when_the_operation_returns_an_error() {
    let notified = Cell::new(false);

    let result = mutate_then_notify(
        || Err::<(), _>(command_error("INJECTED_FAILURE", "injected")),
        || notified.set(true),
    );

    assert_eq!(result.unwrap_err().code, "INJECTED_FAILURE");
    assert!(notified.get());
}

#[test]
fn completed_remote_catalog_revalidates_the_immutable_result_before_history_projection() {
    let dir = temp_dir("completed-remote-catalog");
    let database = dir.join("jobs.sqlite3");
    let source_path = dir.join("meeting.wav");
    let remote_jobs = dir.join("remote-jobs");
    write_pcm_wav(&source_path, &vec![0_u8; 320]);
    let mut source = fs::File::open(&source_path).unwrap();
    let owner = crate::audio::session::OwnerNamespace::local("i-catalog-test").unwrap();
    let prepared = remote::prepare_imported_pcm_wav(
        "job-completed-catalog",
        "meeting.wav",
        &mut source,
        &remote_jobs,
        &owner,
        UNIX_EPOCH + Duration::from_secs(1_720_000_000),
    )
    .unwrap();
    let request = prepared.request.clone();
    let durable = prepared.into_ledger_state().unwrap();
    let ledger = JobLedger::open(&database).unwrap();
    ledger
        .insert_job(&NewRecordingJob {
            job_id: "job-completed-catalog".into(),
            session_mode: SessionMode::Meeting,
            session_origin: SessionOrigin::ImportedFile,
            source_path: Some(source_path.clone()),
            source_ownership: SourceOwnership::External,
            output_path: None,
            display_name: "meeting.wav".into(),
            status: RecordingJobStatus::Preprocessing,
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
    ledger
        .attach_prepared_remote_job("job-completed-catalog", &durable, 1_720_000_000_100)
        .unwrap();
    let server_job_id = "job-0123456789abcdef0123456789abcdef";
    ledger
        .begin_remote_create_attempt(
            "job-completed-catalog",
            "http://127.0.0.1:18765",
            1_720_000_000_200,
        )
        .unwrap();
    ledger
        .record_server_job_id(
            "job-completed-catalog",
            server_job_id,
            "http://127.0.0.1:18765",
            1_720_000_000_200,
        )
        .unwrap();
    for chunk in &request.chunks {
        ledger
            .acknowledge_remote_chunk(
                "job-completed-catalog",
                &chunk.replay_key.track_id,
                chunk.replay_key.sequence_start,
                chunk.replay_key.sequence_end,
                &chunk.content_identity.sha256,
                1_720_000_000_300,
            )
            .unwrap();
    }
    ledger
        .mark_remote_job_committed("job-completed-catalog", 1_720_000_000_400)
        .unwrap();
    ledger
        .begin_remote_result_saving("job-completed-catalog", 1_720_000_000_500)
        .unwrap();
    let result = crate::server_connector::batch::TranscriptResultRevision {
        session_id: request.metadata.session_id.to_string(),
        revision: 1,
        authority: "server_authoritative".into(),
        created_at_utc: "2026-07-14T21:00:02Z".into(),
        capture_manifest_sha256: request.capture_manifest.sha256.clone(),
        previous_result_sha256: None,
        status: "complete".into(),
        language: Some(crate::server_connector::batch::LanguageDecision {
            language_bcp47: "en-US".into(),
            confidence: Some(0.98),
        }),
        transcript: "Catalog result.".into(),
        aligned_words: Vec::new(),
        model_provenance: vec![crate::server_connector::batch::ModelRevision {
            model_id: "CohereLabs/cohere-transcribe-03-2026".into(),
            revision: "b1eacc2686a3d08ceaae5f24a88b1d519620bc09".into(),
            calibration_revision: "asr-not-applicable".into(),
        }],
    };
    let output =
        remote::publish_remote_result("job-completed-catalog", &remote_jobs, &result).unwrap();
    ledger
        .complete_remote_result(
            "job-completed-catalog",
            &output,
            1_722_592_000_000,
            1_720_000_000_600,
        )
        .unwrap();
    let jobs = RecordingJobs::from_ledger(ledger, &dir);

    let catalog = jobs.completed_remote_transcripts().unwrap();
    assert_eq!(catalog.sessions.len(), 1);
    assert_eq!(
        catalog.sessions[0].output_path,
        output.display().to_string()
    );
    assert!(catalog.maintenance_warnings.is_empty());

    fs::write(&output, "tampered\n").unwrap();
    let rejected = jobs.completed_remote_transcripts().unwrap();
    assert!(rejected.sessions.is_empty());
    assert_eq!(rejected.maintenance_warnings.len(), 1);

    assert!(jobs
        .snapshot(&MediaOwner::new(), 1_722_592_000_000)
        .unwrap()
        .is_empty());
    assert!(!remote_jobs.join("job-completed-catalog").exists());
    assert!(
        source_path.is_file(),
        "external source must never be deleted"
    );

    drop(jobs);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn create_imports_validates_and_native_allowlists_a_canonical_recording() {
    let dir = temp_dir("create-import");
    let source = dir.join("meeting.wav");
    fs::write(&source, b"RIFF-command-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();

    let created = jobs
        .create_imports(&media, vec![source.display().to_string()], 1_000)
        .unwrap();

    assert_eq!(created.len(), 1);
    assert_eq!(
        created[0].source_path.as_deref(),
        source.canonicalize().unwrap().to_str()
    );
    assert!(created[0]
        .playback_path
        .as_deref()
        .is_some_and(|path| path.starts_with("http://127.0.0.1:")));
    assert_eq!(created[0].id, jobs.snapshot(&media, 1_001).unwrap()[0].id);
    assert!(fs::read_to_string(&jobs.registry_path)
        .unwrap()
        .contains("meeting.wav"));
    assert!(!dir.join("recording-playback-registry.json").exists());

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn create_imports_rejects_media_that_phase5_cannot_prepare() {
    let dir = temp_dir("create-unsupported-remote-media");
    let source = dir.join("meeting.mp3");
    fs::write(&source, b"not admitted before remote preparation").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();

    let error = jobs
        .create_imports(&media, vec![source.display().to_string()], 1_000)
        .unwrap_err();

    assert_eq!(error.code, "REMOTE_MEDIA_UNSUPPORTED");
    assert!(error.message.contains("mono PCM16 16 kHz WAV"));
    assert!(jobs.snapshot(&media, 1_001).unwrap().is_empty());

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}
