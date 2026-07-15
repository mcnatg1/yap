use super::*;

#[test]
fn create_imports_is_all_or_nothing_for_invalid_paths_and_snapshot_is_stably_ordered() {
    let dir = temp_dir("validation-ordering");
    let later = dir.join("later.wav");
    let earlier = dir.join("earlier.wav");
    let missing = dir.join("missing.wav");
    fs::write(&later, b"RIFF-later-fixture").unwrap();
    fs::write(&earlier, b"RIFF-earlier-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();

    let invalid = jobs.create_imports(
        &media,
        vec![later.display().to_string(), missing.display().to_string()],
        2_500,
    );
    assert_eq!(invalid.unwrap_err().code, "SOURCE_MISSING");
    assert!(jobs.ledger.list_jobs().unwrap().is_empty());
    assert!(!jobs.registry_path.exists());
    assert_eq!(media.active_admission_count_for_test(), 0);

    let later_id = jobs
        .create_imports(&media, vec![later.display().to_string()], 2_700)
        .unwrap()[0]
        .id
        .clone();
    let earlier_id = jobs
        .create_imports(&media, vec![earlier.display().to_string()], 2_600)
        .unwrap()[0]
        .id
        .clone();
    let snapshot = jobs.snapshot(&media, 2_800).unwrap();

    assert_eq!(
        snapshot
            .iter()
            .map(|job| job.id.as_str())
            .collect::<Vec<_>>(),
        [earlier_id.as_str(), later_id.as_str()]
    );
    assert!(snapshot.iter().all(|job| job.id.starts_with("job-")));
    assert!(snapshot.iter().all(|job| job.id.parse::<u64>().is_err()));

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn restart_keeps_a_missing_external_source_visible_but_never_reauthorizes_it() {
    let dir = temp_dir("restart-missing");
    let database = dir.join("jobs.sqlite3");
    let source = dir.join("moved.wav");
    fs::write(&source, b"RIFF-missing-after-restart").unwrap();
    let original_id = {
        let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
        let media = MediaOwner::new();
        let id = jobs
            .create_imports(&media, vec![source.display().to_string()], 3_000)
            .unwrap()[0]
            .id
            .clone();
        drop(media);
        id
    };
    fs::remove_file(&source).unwrap();

    let jobs = RecordingJobs::from_ledger(JobLedger::open(&database).unwrap(), &dir);
    let media = MediaOwner::new();
    let snapshot = jobs.snapshot(&media, 3_001).unwrap();

    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].id, original_id);
    assert_eq!(snapshot[0].status, RecordingJobStatus::Failed);
    assert_eq!(snapshot[0].error.as_deref(), Some("SOURCE_MISSING"));
    assert_eq!(snapshot[0].source_path, None);
    assert_eq!(snapshot[0].playback_path, None);

    drop(media);
    drop(jobs);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn product_bound_counts_existing_recoverable_jobs_across_create_calls() {
    let dir = temp_dir("product-bound");
    let paths = (0..MAX_RECORDING_JOBS)
        .map(|index| {
            let source = dir.join(format!("recording-{index:03}.wav"));
            fs::write(&source, b"RIFF-bound-fixture").unwrap();
            source.display().to_string()
        })
        .collect::<Vec<_>>();
    let overflow = dir.join("overflow.wav");
    fs::write(&overflow, b"RIFF-overflow-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();

    assert_eq!(
        jobs.create_imports(&media, paths, 4_000).unwrap().len(),
        MAX_RECORDING_JOBS
    );
    let admissions_before_overflow = media.active_admission_count_for_test();
    let registry_before_overflow = fs::read(&jobs.registry_path).unwrap();
    let error = jobs
        .create_imports(&media, vec![overflow.display().to_string()], 4_001)
        .unwrap_err();

    assert_eq!(error.code, "JOB_LIMIT_EXCEEDED");
    assert_eq!(
        media.active_admission_count_for_test(),
        admissions_before_overflow
    );
    assert_eq!(
        fs::read(&jobs.registry_path).unwrap(),
        registry_before_overflow
    );
    assert_eq!(
        jobs.snapshot(&media, 4_002).unwrap().len(),
        MAX_RECORDING_JOBS
    );
    assert_eq!(
        media.active_admission_count_for_test(),
        admissions_before_overflow
    );

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn snapshot_expires_pending_jobs_after_seven_days_without_touching_the_source() {
    let dir = temp_dir("pending-expiry");
    let source = dir.join("old.wav");
    fs::write(&source, b"RIFF-expiry-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();
    let created = jobs
        .create_imports(&media, vec![source.display().to_string()], 5_000)
        .unwrap();
    let owned_spool = dir.join("remote-jobs").join(&created[0].id);
    fs::create_dir_all(&owned_spool).unwrap();
    fs::write(owned_spool.join("private.pcm"), b"private copy").unwrap();

    let snapshot = jobs
        .snapshot(&media, 5_000 + PENDING_JOB_LIFETIME_MS)
        .unwrap();

    assert_eq!(snapshot[0].status, RecordingJobStatus::Failed);
    assert_eq!(snapshot[0].error.as_deref(), Some("PENDING_EXPIRED"));
    assert_eq!(snapshot[0].source_path, None);
    assert_eq!(snapshot[0].playback_path, None);
    assert!(source.is_file(), "external source must never be deleted");
    assert!(
        !owned_spool.exists(),
        "expired jobs must delete Yap's private source copy"
    );
    let retried = jobs
        .retry(
            &media,
            &snapshot[0].id,
            5_001 + PENDING_JOB_LIFETIME_MS,
            || {},
        )
        .unwrap();
    assert_eq!(retried.status, RecordingJobStatus::QueuedServer);
    assert_eq!(
        jobs.snapshot(&media, 5_001 + PENDING_JOB_LIFETIME_MS)
            .unwrap()[0]
            .status,
        RecordingJobStatus::QueuedServer
    );

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn snapshot_cancels_expired_active_remote_work_and_deletes_only_the_owned_spool() {
    let dir = temp_dir("active-remote-expiry");
    let source = dir.join("active.wav");
    fs::write(&source, b"RIFF-active-expiry-fixture").unwrap();
    let jobs = RecordingJobs::from_ledger(JobLedger::open_in_memory().unwrap(), &dir);
    let media = MediaOwner::new();
    let created = jobs
        .create_imports(&media, vec![source.display().to_string()], 6_000)
        .unwrap();
    let job_id = &created[0].id;
    jobs.ledger
        .transition(job_id, RecordingJobStatus::Preprocessing, 6_001)
        .unwrap();
    let owned_spool = dir.join("remote-jobs").join(job_id);
    fs::create_dir_all(&owned_spool).unwrap();
    fs::write(owned_spool.join("private.pcm"), b"private copy").unwrap();

    assert!(jobs
        .snapshot(&media, 6_000 + PENDING_JOB_LIFETIME_MS)
        .unwrap()
        .is_empty());
    let expired = jobs.ledger.get_job(job_id).unwrap().unwrap();
    assert_eq!(expired.status, RecordingJobStatus::Cancelled);
    assert!(expired.cancellation_requested);
    assert!(source.is_file(), "external source must never be deleted");
    assert!(
        !owned_spool.exists(),
        "expired active work must delete Yap's private source copy"
    );

    drop(media);
    fs::remove_dir_all(dir).unwrap();
}
