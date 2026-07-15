use super::*;

#[test]
fn authority_failed_create_stays_capability_free_until_explicit_retry() {
    let dir = temp_dir("create-admission-failure");
    let database = dir.join("jobs.sqlite3");
    let general_registry = dir.join("recording-playback-registry.json");
    let source = dir.join("meeting.wav");
    let source_bytes = b"RIFF-command-fixture";
    fs::write(&source, source_bytes).unwrap();
    let canonical_source = source.canonicalize().unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
    let media = MediaOwner::new();
    jobs.inject_projection_failures_for_test(vec![command_error(
        "PLAYBACK_AUTHORITY_FAILED",
        "injected admission failure",
    )]);
    let authority_denied_before_event_snapshot = Cell::new(false);
    let authority_denied_after_event_snapshot = Cell::new(false);
    let event_snapshot = RefCell::new(None);

    let created = mutate_then_notify(
        || jobs.create_imports(&media, vec![source.display().to_string()], 1_500),
        || {
            authority_denied_before_event_snapshot.set(open_and_reveal_are_denied(
                &jobs,
                &source,
                &general_registry,
            ));
            let snapshot = jobs.snapshot(&media, 1_501).unwrap();
            authority_denied_after_event_snapshot.set(open_and_reveal_are_denied(
                &jobs,
                &source,
                &general_registry,
            ));
            *event_snapshot.borrow_mut() = Some(snapshot);
        },
    )
    .unwrap();
    let event_snapshot = event_snapshot.into_inner().unwrap();
    let duplicate = jobs
        .create_imports(&media, vec![source.display().to_string()], 1_502)
        .unwrap();
    let duplicate_authority_denied = open_and_reveal_are_denied(&jobs, &source, &general_registry);
    let committed = jobs.ledger().get_job(&created[0].id).unwrap().unwrap();

    assert_eq!(
        committed.error_code.as_deref(),
        Some("PLAYBACK_AUTHORITY_FAILED")
    );
    assert_eq!(
        committed.source_path.as_deref(),
        Some(canonical_source.as_path())
    );
    assert_eq!(duplicate[0].id, created[0].id);
    assert_eq!(fs::read(&source).unwrap(), source_bytes);

    drop(media);
    drop(jobs);

    let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
    let media = MediaOwner::new();
    let restart_snapshot = jobs.snapshot(&media, 1_503).unwrap();
    let restart_authority_denied = open_and_reveal_are_denied(&jobs, &source, &general_registry);
    let restarted = jobs.ledger().get_job(&created[0].id).unwrap().unwrap();
    assert_eq!(
        restarted.source_path.as_deref(),
        Some(canonical_source.as_path())
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);

    let observations = [
        ("immediate response", &created[0]),
        ("event snapshot", &event_snapshot[0]),
        ("duplicate create", &duplicate[0]),
        ("restart snapshot", &restart_snapshot[0]),
    ];
    let authority_denials = [
        (
            "before event snapshot",
            authority_denied_before_event_snapshot.get(),
        ),
        (
            "after event snapshot",
            authority_denied_after_event_snapshot.get(),
        ),
        ("after duplicate create", duplicate_authority_denied),
        ("after restart snapshot", restart_authority_denied),
    ];
    assert!(
        authority_denials.iter().all(|(_, denied)| *denied),
        "open/reveal authorization must remain denied: {authority_denials:#?}"
    );
    assert!(
        observations
            .iter()
            .all(|(_, view)| capability_free_failed(view)),
        "every durable failed projection must be capability-free: {observations:#?}"
    );

    let retried = jobs.retry(&media, &created[0].id, 1_504, || {}).unwrap();
    assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
    assert_eq!(retried.source_path.as_deref(), canonical_source.to_str());
    assert!(retried.playback_path.is_some());
    assert_eq!(
        crate::recording_access::openable_app_path_from_registries(
            source.display().to_string(),
            &general_registry,
            &jobs.registry_path,
            jobs.owned_dir(),
        )
        .unwrap(),
        canonical_source
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);

    drop(media);
    drop(jobs);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pre_native_picker_ledger_and_registry_cannot_reauthorize_on_restart() {
    let dir = temp_dir("pre-native-picker-restart");
    let database = dir.join("jobs.sqlite3");
    let source = dir.join("legacy-renderer-path.wav");
    fs::write(&source, b"RIFF-pre-native-picker-fixture").unwrap();
    let canonical_source = source.canonicalize().unwrap();
    let job_id;
    let active_registry;
    let selection_registry;
    {
        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        job_id = jobs
            .create_imports(&media, vec![source.clone()], 1_600)
            .unwrap()[0]
            .id
            .clone();
        active_registry = jobs.registry_path.clone();
        selection_registry = jobs.selection_registry_path.clone();
    }
    fs::remove_file(&selection_registry).unwrap();
    fs::write(
        &active_registry,
        format!(
            r#"{{"version":1,"paths":[{}]}}"#,
            serde_json::to_string(&canonical_source.display().to_string()).unwrap()
        ),
    )
    .unwrap();

    let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
    let media = MediaOwner::new();
    let snapshot = jobs.snapshot(&media, 1_601).unwrap();

    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].id, job_id);
    assert!(capability_free_failed(&snapshot[0]));
    assert_eq!(media.active_admission_count_for_test(), 0);
    assert!(crate::recording_access::openable_app_path_from_registries(
        source.display().to_string(),
        &dir.join("recording-playback-registry.json"),
        &jobs.registry_path,
        jobs.owned_dir(),
    )
    .is_err());
    drop(media);
    drop(jobs);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn multi_create_commits_every_row_before_returning_injected_projection_outcomes() {
    let dir = temp_dir("multi-create-admission-failure");
    let failed_source = dir.join("failed.wav");
    let queued_source = dir.join("queued.wav");
    fs::write(&failed_source, b"RIFF-failed-fixture").unwrap();
    fs::write(&queued_source, b"RIFF-queued-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();
    jobs.inject_projection_failures_for_test(vec![command_error(
        "PLAYBACK_AUTHORITY_FAILED",
        "injected first-row admission failure",
    )]);

    let created = mutate_then_notify(
        || {
            jobs.create_imports(
                &media,
                vec![
                    failed_source.display().to_string(),
                    queued_source.display().to_string(),
                ],
                1_700,
            )
        },
        || {
            let committed = jobs.ledger().list_recoverable_jobs().unwrap();
            assert_eq!(committed.len(), 2);
            assert_eq!(
                committed
                    .iter()
                    .filter(|job| job.status == RecordingJobStatus::Failed)
                    .count(),
                1
            );
            assert_eq!(
                committed
                    .iter()
                    .filter(|job| job.status == RecordingJobStatus::QueuedServer)
                    .count(),
                1
            );
        },
    )
    .unwrap();

    assert_eq!(created.len(), 2);
    assert_eq!(created[0].status, RecordingJobStatus::Failed);
    assert_eq!(created[0].playback_path, None);
    assert_eq!(created[1].status, RecordingJobStatus::QueuedServer);
    assert!(created[1].playback_path.is_some());

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn duplicate_file_import_returns_the_existing_rust_minted_job() {
    let dir = temp_dir("duplicate-import");
    let source = dir.join("same.wav");
    fs::write(&source, b"RIFF-duplicate-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();
    let path = source.display().to_string();

    let first = jobs
        .create_imports(&media, vec![path.clone()], 2_000)
        .unwrap();
    let duplicate = jobs.create_imports(&media, vec![path], 2_001).unwrap();

    assert_eq!(duplicate[0].id, first[0].id);
    assert_eq!(duplicate[0].playback_path, first[0].playback_path);
    assert_eq!(media.active_admission_count_for_test(), 1);
    assert_eq!(jobs.snapshot(&media, 2_002).unwrap().len(), 1);
    assert_eq!(media.active_admission_count_for_test(), 1);

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unrelated_mutations_preserve_playback_but_source_replacement_rotates_it() {
    let dir = temp_dir("stable-playback");
    let selected = dir.join("selected.wav");
    let unrelated = dir.join("unrelated.wav");
    let original = dir.join("selected-original.wav");
    fs::write(&selected, b"RIFF-selected-original").unwrap();
    fs::write(&unrelated, b"RIFF-unrelated").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();

    let selected_job = jobs
        .create_imports(&media, vec![selected.display().to_string()], 2_100)
        .unwrap()[0]
        .clone();
    jobs.create_imports(&media, vec![unrelated.display().to_string()], 2_101)
        .unwrap();
    let after_unrelated = jobs.snapshot(&media, 2_102).unwrap();
    let selected_after_unrelated = after_unrelated
        .iter()
        .find(|job| job.id == selected_job.id)
        .unwrap();
    assert_eq!(
        selected_after_unrelated.playback_path,
        selected_job.playback_path
    );
    assert_eq!(media.active_admission_count_for_test(), 2);

    fs::rename(&selected, &original).unwrap();
    fs::write(&selected, b"RIFF-selected-replacement").unwrap();
    let after_replacement = jobs.snapshot(&media, 2_103).unwrap();
    let selected_after_replacement = after_replacement
        .iter()
        .find(|job| job.id == selected_job.id)
        .unwrap();
    assert_ne!(
        selected_after_replacement.playback_path,
        selected_job.playback_path
    );
    assert_eq!(media.active_admission_count_for_test(), 2);

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}
