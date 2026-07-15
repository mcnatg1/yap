use super::*;

#[test]
fn cancellation_and_dismissal_delete_only_yap_owned_remote_spools() {
    let dir = temp_dir("remote-terminal-cleanup");
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();

    let cancel_source = dir.join("cancel.wav");
    fs::write(&cancel_source, b"RIFF-cancel-fixture").unwrap();
    let cancelled = jobs
        .create_imports(&media, vec![cancel_source.display().to_string()], 7_000)
        .unwrap();
    let cancel_spool = dir.join("remote-jobs").join(&cancelled[0].id);
    fs::create_dir_all(&cancel_spool).unwrap();
    fs::write(cancel_spool.join("private.pcm"), b"private copy").unwrap();
    jobs.cancel(&media, &cancelled[0].id, 7_001, || {}).unwrap();
    assert!(cancel_source.is_file());
    assert!(!cancel_spool.exists());

    let dismiss_source = dir.join("dismiss.wav");
    fs::write(&dismiss_source, b"RIFF-dismiss-fixture").unwrap();
    let dismissed = jobs
        .create_imports(&media, vec![dismiss_source.display().to_string()], 7_100)
        .unwrap();
    jobs.ledger()
        .fail_source_validation(&dismissed[0].id, "TEST_FAILED", 7_101)
        .unwrap();
    let dismiss_spool = dir.join("remote-jobs").join(&dismissed[0].id);
    fs::create_dir_all(&dismiss_spool).unwrap();
    fs::write(dismiss_spool.join("private.pcm"), b"private copy").unwrap();
    jobs.dismiss(&media, &dismissed[0].id, 7_102, || {})
        .unwrap();
    assert!(dismiss_source.is_file());
    assert!(!dismiss_spool.exists());

    let retry_source = dir.join("retry.wav");
    fs::write(&retry_source, b"RIFF-retry-fixture").unwrap();
    let retried = jobs
        .create_imports(&media, vec![retry_source.display().to_string()], 7_200)
        .unwrap();
    jobs.ledger()
        .fail_source_validation(&retried[0].id, "TEST_FAILED", 7_201)
        .unwrap();
    let retry_spool = dir.join("remote-jobs").join(&retried[0].id);
    fs::create_dir_all(&retry_spool).unwrap();
    fs::write(retry_spool.join("private.pcm"), b"private copy").unwrap();
    jobs.retry(&media, &retried[0].id, 7_202, || {}).unwrap();
    assert!(retry_source.is_file());
    assert!(!retry_spool.exists());

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dismissing_failed_jobs_preserves_provenance_and_frees_capacity_after_restart() {
    let dir = temp_dir("dismiss-capacity");
    let database = dir.join("jobs.sqlite3");
    let paths = (0..MAX_RECORDING_JOBS)
        .map(|index| {
            let source = dir.join(format!("failed-{index:03}.wav"));
            fs::write(&source, b"RIFF-failed-fixture").unwrap();
            source
        })
        .collect::<Vec<_>>();
    let replacement = dir.join("replacement.wav");
    fs::write(&replacement, b"RIFF-replacement-fixture").unwrap();

    {
        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let created = jobs
            .create_imports(
                &media,
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
                5_500,
            )
            .unwrap();
        for (index, job) in created.iter().enumerate() {
            jobs.ledger()
                .fail_source_validation(&job.id, "TEST_FAILED", 5_600 + index as u64)
                .unwrap();
            jobs.dismiss(&media, &job.id, 5_900 + index as u64, || {})
                .unwrap();
        }

        assert!(jobs.snapshot(&media, 6_200).unwrap().is_empty());
        assert_eq!(media.active_admission_count_for_test(), 0);
        let durable = jobs.ledger().list_jobs().unwrap();
        assert_eq!(durable.len(), MAX_RECORDING_JOBS);
        assert!(durable.iter().all(|job| {
            job.status == RecordingJobStatus::Cancelled
                && job.source_path.is_some()
                && job.error_code.as_deref() == Some("TEST_FAILED")
                && job.cancellation_requested
        }));
    }

    assert!(paths.iter().all(|path| path.is_file()));
    let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
    let media = MediaOwner::new();
    assert!(jobs.snapshot(&media, 6_300).unwrap().is_empty());
    let imported = jobs
        .create_imports(&media, vec![replacement.display().to_string()], 6_301)
        .unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].status, RecordingJobStatus::QueuedServer);

    drop(media);
    drop(jobs);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn more_than_five_hundred_terminal_imports_do_not_exhaust_job_path_authority() {
    let dir = temp_dir("terminal-authority-cycles");
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();
    let mut sources = Vec::new();

    for index in 0..=500 {
        let source = dir.join(format!("terminal-{index:03}.wav"));
        fs::write(&source, b"RIFF-terminal-authority-fixture").unwrap();
        let created = jobs
            .create_imports(
                &media,
                vec![source.display().to_string()],
                6_500 + index as u64 * 3,
            )
            .unwrap();
        assert_eq!(created[0].status, RecordingJobStatus::QueuedServer);
        if index % 2 == 0 {
            jobs.cancel(&media, &created[0].id, 6_501 + index as u64 * 3, || {})
                .unwrap();
        } else {
            jobs.ledger()
                .fail_source_validation(&created[0].id, "TEST_FAILED", 6_501 + index as u64 * 3)
                .unwrap();
            jobs.dismiss(&media, &created[0].id, 6_502 + index as u64 * 3, || {})
                .unwrap();
        }
        sources.push(source);
    }

    let final_source = dir.join("still-authorized.wav");
    fs::write(&final_source, b"RIFF-final-authority-fixture").unwrap();
    let final_import = jobs
        .create_imports(&media, vec![final_source.display().to_string()], 8_100)
        .unwrap();

    assert_eq!(final_import[0].status, RecordingJobStatus::QueuedServer);
    assert!(sources.iter().all(|source| source.is_file()));

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_job_authority_is_removed_without_harming_general_authority_or_bytes() {
    let dir = temp_dir("terminal-authority-removal");
    let general_registry = dir.join("recording-playback-registry.json");
    let cancelled_source = dir.join("cancelled.wav");
    let dismissed_source = dir.join("dismissed.wav");
    let general_source = dir.join("general.wav");
    for source in [&cancelled_source, &dismissed_source, &general_source] {
        fs::write(source, b"RIFF-terminal-authority-fixture").unwrap();
    }
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();

    let cancelled = jobs
        .create_imports(&media, vec![cancelled_source.display().to_string()], 8_200)
        .unwrap();
    jobs.cancel(&media, &cancelled[0].id, 8_201, || {}).unwrap();
    assert!(crate::file_actions::openable_app_path_from_registries(
        cancelled_source.display().to_string(),
        &general_registry,
        &jobs.registry_path,
        jobs.owned_dir(),
    )
    .is_err());

    let dismissed = jobs
        .create_imports(&media, vec![dismissed_source.display().to_string()], 8_202)
        .unwrap();
    jobs.ledger()
        .fail_source_validation(&dismissed[0].id, "TEST_FAILED", 8_203)
        .unwrap();
    jobs.dismiss(&media, &dismissed[0].id, 8_204, || {})
        .unwrap();
    assert!(crate::file_actions::openable_app_path_from_registries(
        dismissed_source.display().to_string(),
        &general_registry,
        &jobs.registry_path,
        jobs.owned_dir(),
    )
    .is_err());

    let general = jobs
        .create_imports(&media, vec![general_source.display().to_string()], 8_205)
        .unwrap();
    crate::file_actions::register_general_playback_path_at_for_test(
        general_source.display().to_string(),
        &general_registry,
        jobs.owned_dir(),
    )
    .unwrap();
    jobs.ledger()
        .fail_source_validation(&general[0].id, "TEST_FAILED", 8_206)
        .unwrap();
    jobs.dismiss(&media, &general[0].id, 8_207, || {}).unwrap();
    assert_eq!(
        crate::file_actions::openable_app_path_from_registries(
            general_source.display().to_string(),
            &general_registry,
            &jobs.registry_path,
            jobs.owned_dir(),
        )
        .unwrap(),
        general_source.canonicalize().unwrap()
    );
    assert!([&cancelled_source, &dismissed_source, &general_source]
        .iter()
        .all(|source| source.is_file()));

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn restart_snapshot_prunes_job_authority_left_by_a_terminal_commit() {
    let dir = temp_dir("restart-authority-prune");
    let database = dir.join("jobs.sqlite3");
    let general_registry = dir.join("recording-playback-registry.json");
    let source = dir.join("stale.wav");
    fs::write(&source, b"RIFF-stale-authority-fixture").unwrap();

    {
        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let created = jobs
            .create_imports(&media, vec![source.display().to_string()], 8_300)
            .unwrap();
        assert!(crate::file_actions::openable_app_path_from_registries(
            source.display().to_string(),
            &general_registry,
            &jobs.registry_path,
            jobs.owned_dir(),
        )
        .is_ok());
        jobs.ledger()
            .request_cancellation(&created[0].id, 8_301)
            .unwrap();
    }

    let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
    let media = MediaOwner::new();
    assert!(jobs.snapshot(&media, 8_302).unwrap().is_empty());
    assert!(crate::file_actions::openable_app_path_from_registries(
        source.display().to_string(),
        &general_registry,
        &jobs.registry_path,
        jobs.owned_dir(),
    )
    .is_err());
    assert!(source.is_file());

    drop(media);
    drop(jobs);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn terminal_registry_cleanup_failure_does_not_hide_the_committed_transition() {
    let dir = temp_dir("terminal-cleanup-failure");
    let source = dir.join("cleanup.wav");
    fs::write(&source, b"RIFF-cleanup-failure-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();
    let created = jobs
        .create_imports(&media, vec![source.display().to_string()], 8_400)
        .unwrap();
    fs::remove_file(&jobs.registry_path).unwrap();
    fs::create_dir(&jobs.registry_path).unwrap();

    let cancelled = mutate_then_notify(
        || jobs.cancel(&media, &created[0].id, 8_401, || {}),
        || {
            assert_eq!(
                jobs.ledger()
                    .get_job(&created[0].id)
                    .unwrap()
                    .unwrap()
                    .status,
                RecordingJobStatus::Cancelled
            );
        },
    )
    .unwrap();

    assert_eq!(cancelled.status, RecordingJobStatus::Cancelled);
    assert!(jobs.snapshot(&media, 8_402).unwrap().is_empty());
    assert!(source.is_file());

    fs::remove_dir(&jobs.registry_path).unwrap();
    drop(media);
    fs::remove_dir_all(dir).unwrap();
}
