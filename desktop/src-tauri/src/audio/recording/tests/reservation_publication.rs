use super::*;

#[test]
fn journal_create_failure_leaves_wav_part_as_a_recovery_candidate() {
    let dir = tempfile_dir("journal-create-failure");
    let session = SessionId::new("s-journal-create-failure").unwrap();
    std::fs::write(
        dir.join(format!("live-{session}.capture.journal.part")),
        "occupied",
    )
    .unwrap();

    assert!(StreamingRecording::create(&dir, session.clone()).is_err());

    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);
    assert_eq!(scan.partial[0].session_id.as_ref(), Some(&session));
}

#[test]
fn journal_append_keeps_stale_unowned_files_untouched() {
    let dir = tempfile_dir("journal-private-temp");
    let session = SessionId::new("s-journal-private-temp").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    let stale = dir.join(format!("live-{session}.capture.journal.part.next"));
    std::fs::write(&stale, b"stale private snapshot").unwrap();

    recording.persist_journal_for_test().unwrap();

    assert_eq!(std::fs::read(&stale).unwrap(), b"stale private snapshot");
    drop(recording);
    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn persistent_allocation_survives_runtime_restarts_without_reusing_numeric_names() {
    let dir = tempfile_dir("persistent-allocation-restart");
    let first = allocate_recording_session(&dir).unwrap();
    let first_session = first.session_id().clone();
    let mut first_recording = StreamingRecording::create_reserved(first).unwrap();
    first_recording.append_pcm16(&[1, 0]).unwrap();
    first_recording.finalize().unwrap();

    let second = allocate_recording_session(&dir).unwrap();

    assert_ne!(first_session, *second.session_id());
    assert!(dir
        .join(format!("live-{first_session}.commit.json"))
        .is_file());
    assert!(dir
        .join(format!("live-{}.wav.part", second.session_id()))
        .is_file());
}

#[test]
fn reservation_handoff_never_adopts_a_replaced_wav_part_path() {
    #[derive(Clone, Copy)]
    enum Replacement {
        Delete,
        Regular,
        HardLink,
        Reparse,
    }

    for replacement in [
        Replacement::Delete,
        Replacement::Regular,
        Replacement::HardLink,
        Replacement::Reparse,
    ] {
        let dir = tempfile_dir("reservation-worker-handoff");
        let mut reservation = allocate_recording_session(&dir).unwrap();
        let session_id = reservation.session_id().clone();
        let wav_part = reservation.wav_part().to_path_buf();
        let replacement_path = dir.join("attacker-replacement");
        let handle_dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        reservation.watch_handle_drop_for_test(std::sync::Arc::clone(&handle_dropped));
        fs::remove_file(&wav_part).unwrap();

        let replacement_bytes = match replacement {
            Replacement::Delete => None,
            Replacement::Regular => {
                let bytes = b"attacker regular replacement".to_vec();
                fs::write(&wav_part, &bytes).unwrap();
                Some((wav_part.clone(), bytes))
            }
            Replacement::HardLink => {
                let bytes = b"attacker hard-link replacement".to_vec();
                fs::write(&replacement_path, &bytes).unwrap();
                fs::hard_link(&replacement_path, &wav_part).unwrap();
                Some((wav_part.clone(), bytes))
            }
            Replacement::Reparse => {
                let bytes = b"attacker reparse replacement".to_vec();
                fs::write(&replacement_path, &bytes).unwrap();
                if let Err(error) = create_file_symlink_for_test(&replacement_path, &wav_part) {
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        || error.raw_os_error() == Some(1314)
                    {
                        fs::write(&wav_part, &bytes).unwrap();
                        Some((wav_part.clone(), bytes))
                    } else {
                        panic!("failed to install reparse replacement: {error}");
                    }
                } else {
                    Some((wav_part.clone(), bytes))
                }
            }
        };

        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        let handle = RecordingSinkHandle::spawn_reserved(reservation, sink, receiver);
        let result = handle.finalize().unwrap();

        assert_eq!(result.status, CaptureStatus::Partial);
        assert!(result.committed.is_none());
        assert!(!dir.join(format!("live-{session_id}.commit.json")).exists());
        let scan = scan_recordings(&dir).unwrap();
        assert!(scan.complete.is_empty());
        assert!(!scan.partial.is_empty());
        if let Some((path, bytes)) = replacement_bytes {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert!(handle_dropped.load(std::sync::atomic::Ordering::SeqCst));
        std::fs::remove_file(&wav_part).ok();
        std::fs::remove_file(&replacement_path).ok();
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn reservation_rejects_every_preexisting_artifact_for_the_same_session() {
    let session = SessionId::new("s-existing-artifact").unwrap();
    for suffix in [
        ".wav",
        ".txt",
        ".capture.json",
        ".capture.partial.json",
        ".commit.json",
        ".transcript.r1.json",
        ".capture.journal.part",
    ] {
        let dir = tempfile_dir(&format!("preexisting-{}", suffix.replace('.', "-")));
        std::fs::write(dir.join(format!("live-{session}{suffix}")), b"existing").unwrap();

        let error = reserve_wav_part(&dir, &session).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{suffix}");
        assert!(!dir.join(format!("live-{session}.wav.part")).exists());
    }
}

#[test]
fn reservation_collision_after_preflight_never_removes_the_competing_claim() {
    let dir = tempfile_dir("reservation-race");
    let session = SessionId::new("s-reservation-race").unwrap();
    let claimed = RecordingPaths::new(&dir, session.clone()).wav_part;
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let (claimed_tx, claimed_rx) = std::sync::mpsc::channel();
    let racer_dir = dir.clone();
    let racer_session = session.clone();
    let racer_barrier = Arc::clone(&barrier);
    let racer = std::thread::spawn(move || {
        racer_barrier.wait();
        std::fs::write(
            RecordingPaths::new(&racer_dir, racer_session).wav_part,
            b"racer reservation",
        )
        .unwrap();
        claimed_tx.send(()).unwrap();
    });

    let error = reserve_wav_part_with_before_claim(&dir, &session, || {
        barrier.wait();
        claimed_rx.recv().unwrap();
    })
    .unwrap_err();
    racer.join().unwrap();

    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(claimed).unwrap(), b"racer reservation");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn concurrent_persistent_allocations_reserve_distinct_wav_parts() {
    let dir = tempfile_dir("concurrent-persistent-allocation");
    let barrier = Arc::new(std::sync::Barrier::new(5));
    let mut workers = Vec::new();
    for _ in 0..4 {
        let directory = dir.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            allocate_recording_session(&directory).unwrap()
        }));
    }
    barrier.wait();
    let sessions = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().session_id().to_string())
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(sessions.len(), 4);
    for session in sessions {
        assert!(dir.join(format!("live-{session}.wav.part")).is_file());
    }
}

#[test]
fn collision_safe_publication_never_overwrites_an_existing_artifact() {
    let dir = tempfile_dir("no-overwrite-publication");
    for suffix in [
        ".wav",
        ".capture.json",
        ".capture.partial.json",
        ".commit.json",
        ".txt",
        ".transcript.r1.json",
    ] {
        let source = dir.join(format!("staged{suffix}.part"));
        let destination = dir.join(format!("live-s-safe{suffix}"));
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&destination, b"old").unwrap();
        let owned = File::open(&source).unwrap();

        assert!(publish_no_replace(&source, &destination, &owned, "test publish").is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
    }
}

#[test]
fn hard_link_cleanup_debt_keeps_publication_complete_and_staging_private() {
    for (fault, staging_suffix) in [
        (CommitFaultPoint::AudioStagingCleanup, ".wav.part"),
        (CommitFaultPoint::CommitStagingCleanup, ".commit.json.part"),
    ] {
        let dir = tempfile_dir(&format!("post-link-cleanup-{fault:?}"));
        let session = SessionId::new("s-post-link-cleanup").unwrap();
        let mut recording =
            StreamingRecording::create_with_fault(&dir, session.clone(), fault).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();

        let result = recording.finalize().unwrap();
        assert_eq!(result.status, CaptureStatus::Complete, "{fault:?}");
        assert!(result.committed.is_some(), "{fault:?}");
        let scan = scan_recordings(&dir).unwrap();
        assert_eq!(scan.complete.len(), 1, "{fault:?}");
        assert!(scan.partial.is_empty(), "{fault:?}");
        let staging = dir.join(format!("live-{session}{staging_suffix}"));
        assert!(staging.is_file(), "{fault:?}");
        let owned = File::open(&staging).unwrap();
        assert!(
            publish_no_replace(
                &staging,
                &dir.join(format!("live-{session}.commit.json")),
                &owned,
                "retry"
            )
            .is_err(),
            "{fault:?} must never overwrite the published destination"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
