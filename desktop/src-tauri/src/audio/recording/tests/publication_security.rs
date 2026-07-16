use super::*;

#[test]
fn sidecar_replacement_between_receipt_and_commit_fails_closed() {
    let dir = tempfile_dir("partial-receipt-reparse-replacement");
    let session = SessionId::new("s-partial-receipt-reparse-replacement").unwrap();
    let outside = dir.join("outside-sidecar.json");
    fs::write(&outside, b"attacker sidecar").unwrap();
    let mut recording =
        StreamingRecording::create_with_sidecar_hook(&dir, session.clone(), move |paths| {
            if let Err(error) = fs::remove_file(&paths.sidecar) {
                panic!("failed to remove owned sidecar in test: {error}");
            }
            if let Err(error) = create_file_symlink_for_test(&outside, &paths.sidecar) {
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    || error.raw_os_error() == Some(1314)
                {
                    fs::write(&paths.sidecar, b"attacker replacement").unwrap();
                    return;
                }
                panic!("failed to replace sidecar with reparse point: {error}");
            }
        })
        .unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();

    let result = recording.finalize().unwrap();
    let lineage = result
        .partial_lineage
        .expect("replacement-safe partial lineage");

    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert_eq!(
        lineage.capture_sidecar_file,
        format!("live-{session}.capture.partial.json")
    );
    assert!(
        !dir.join(format!("live-{session}.commit.json")).exists(),
        "a divergent sidecar must never receive a complete commit"
    );
    assert!(scan_recordings(&dir).unwrap().complete.is_empty());
}

#[test]
fn sidecar_replacement_during_commit_publication_fails_closed() {
    for barrier in [
        PublicationBarrier::BeforeHardLink,
        PublicationBarrier::AfterHardLink,
    ] {
        let dir = tempfile_dir(&format!("sidecar-commit-{barrier:?}"));
        let session = SessionId::new("s-sidecar-commit-replacement").unwrap();
        let mut recording = StreamingRecording::create_with_publication_hook(
            &dir,
            session.clone(),
            None,
            move |artifact, reached, paths| {
                if artifact != PublicationArtifact::Commit || reached != barrier {
                    return;
                }
                let displaced = paths
                    .sidecar
                    .with_extension(format!("{barrier:?}.displaced"));
                fs::rename(&paths.sidecar, displaced).unwrap();
                fs::write(&paths.sidecar, b"attacker sidecar").unwrap();
            },
        )
        .unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();

        let result = recording.finalize().unwrap();

        assert_eq!(result.status, CaptureStatus::Partial, "{barrier:?}");
        assert!(result.committed.is_none(), "{barrier:?}");
        assert!(
            scan_recordings(&dir).unwrap().complete.is_empty(),
            "{barrier:?}"
        );
    }
}

#[test]
fn nofollow_handle_keeps_the_original_bytes_across_path_replacement() {
    let dir = tempfile_dir("nofollow-replacement");
    let name = "safe.json";
    let path = dir.join(name);
    fs::write(&path, b"owned bytes").unwrap();
    let mut file = open_regular_artifact(&dir, name).unwrap();
    let displaced = dir.join("displaced-safe.json");
    fs::rename(&path, &displaced).unwrap();
    fs::write(&path, b"attacker bytes").unwrap();
    let mut bytes = String::new();

    file.read_to_string(&mut bytes).unwrap();

    assert_eq!(bytes, "owned bytes");
}

#[test]
fn aborted_capture_open_is_partial_but_successful_zero_duration_is_complete() {
    let failed_dir = tempfile_dir("capture-open-failed");
    let failed_session = SessionId::new("s-capture-open-failed").unwrap();
    let (failed_sink, failed_rx) = bounded_sink(SinkKind::Recording, 1);
    let failed = RecordingSinkHandle::spawn(
        failed_dir.clone(),
        failed_session.clone(),
        failed_sink,
        failed_rx,
    );

    let failed_result = failed.abort("capture adapter could not open").unwrap();

    assert_eq!(failed_result.status, CaptureStatus::Partial);
    assert!(failed_result.committed.is_none());
    assert_eq!(scan_recordings(&failed_dir).unwrap().complete.len(), 0);
    assert_eq!(scan_recordings(&failed_dir).unwrap().partial.len(), 1);

    let complete_dir = tempfile_dir("capture-zero-duration");
    let complete_session = SessionId::new("s-capture-zero-duration").unwrap();
    let (complete_sink, complete_rx) = bounded_sink(SinkKind::Recording, 1);
    let complete = RecordingSinkHandle::spawn(
        complete_dir.clone(),
        complete_session,
        complete_sink,
        complete_rx,
    );

    assert_eq!(complete.finalize().unwrap().status, CaptureStatus::Complete);
    assert_eq!(scan_recordings(&complete_dir).unwrap().complete.len(), 1);
}

#[test]
fn scan_classifies_symlinked_committed_artifacts_as_damaged_when_links_are_supported() {
    let dir = tempfile_dir("symlinked-artifact");
    let session = SessionId::new("s-symlinked-artifact").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();

    let audio = dir.join(format!("live-{session}.wav"));
    let outside = std::env::temp_dir().join(format!(
        "yap-recording-symlink-target-{}-{session}.wav",
        std::process::id()
    ));
    std::fs::remove_file(&outside).ok();
    std::fs::rename(&audio, &outside).unwrap();
    if let Err(error) = create_file_symlink_for_test(&outside, &audio) {
        if error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(1314)
        {
            std::fs::rename(&outside, &audio).unwrap();
            return;
        }
        panic!("failed to create symlink: {error}");
    }

    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert!(scan.partial.is_empty());
    assert_eq!(scan.damaged.len(), 1);
    assert_eq!(scan.damaged[0].session_id, session);
    std::fs::remove_file(&audio).ok();
    std::fs::remove_file(&outside).ok();
}

#[test]
fn committed_metadata_is_hash_bound_and_uses_the_reserved_session_id() {
    let dir = tempfile_dir("metadata-bound");
    let session = SessionId::new("s-metadata-bound").unwrap();
    let metadata = SessionMetadata::new(
        session.clone(),
        SessionMode::Meeting,
        SessionOrigin::LiveCapture,
        TriggerMode::Toggle,
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100),
        None,
        None,
        None,
        Vec::new(),
        Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(200)),
    )
    .unwrap();
    let mut recording = StreamingRecording::create_with_session_metadata(&dir, metadata).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording.finalize().unwrap().committed.unwrap().manifest;

    assert_eq!(
        manifest.session_metadata.as_ref().unwrap().session_id,
        session
    );
    assert_eq!(
        manifest.session_metadata.as_ref().unwrap().mode,
        SessionMode::Meeting
    );
    assert_eq!(scan_recordings(&dir).unwrap().complete.len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovery_rejects_invalid_wav_without_mutating_it() {
    let dir = tempfile_dir("invalid-recovery-wav");
    let session = SessionId::new("s-invalid-recovery-wav").unwrap();
    let path = dir.join(format!("live-{session}.wav.part"));
    let bytes = b"RIFF not a valid Yap wav".to_vec();
    std::fs::write(&path, &bytes).unwrap();

    assert!(recover_partial_wav_for_test(&dir, &session).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert!(!dir.join(format!("live-{session}.wav")).exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn current_writer_final_wav_with_partial_lineage_remains_recoverable() {
    let dir = tempfile_dir("orphan-recovery-wav");
    let session = SessionId::generate().unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.abort("inject partial".into());

    recover_partial_wav_for_test(&dir, &session).unwrap();
    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert!(scan
        .partial
        .iter()
        .any(|partial| partial.session_id.as_ref() == Some(&session)));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn timestamp_named_final_wav_is_not_a_private_partial_candidate() {
    let dir = tempfile_dir("legacy-final-wav");
    let legacy = dir.join("live-1720656000000.wav");
    let mut file = File::create(&legacy).unwrap();
    write_wav_header(&mut file, 2).unwrap();
    file.write_all(&[0, 0]).unwrap();
    file.sync_all().unwrap();

    assert!(scan_recordings(&dir).unwrap().is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn deletion_quarantine_preserves_a_replacement() {
    let dir = tempfile_dir("delete-replacement");
    let name = "live-s-delete-replacement.wav.part";
    let path = dir.join(name);
    std::fs::write(&path, b"owned").unwrap();

    let error = remove_regular_artifact_with_barrier_for_test(&dir, name, |target| {
        let displaced = target.with_extension("displaced");
        std::fs::rename(target, displaced).unwrap();
        std::fs::write(target, b"replacement").unwrap();
    })
    .unwrap_err();

    assert!(error.contains("no longer names the verified file"));
    assert_eq!(std::fs::read(path).unwrap(), b"replacement");
    std::fs::remove_dir_all(dir).ok();
}
