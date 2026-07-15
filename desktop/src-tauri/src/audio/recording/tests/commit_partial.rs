use super::*;

#[test]
fn streamed_pcm_finalizes_only_after_a_commit_manifest() {
    let dir = tempfile_dir("commit-last");
    let session = SessionId::new("s-commit-last").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0, 2, 0]).unwrap();

    let pending = scan_recordings(&dir).unwrap();
    assert!(pending.complete.is_empty());
    assert_eq!(pending.partial.len(), 1);

    let completed = recording.finalize().unwrap();
    assert_eq!(
        completed.status,
        CaptureStatus::Complete,
        "{:?}",
        completed.error
    );
    assert_eq!(scan_recordings(&dir).unwrap().len(), 1);
    assert!(dir.join(format!("live-{session}.commit.json")).is_file());
}

#[test]
fn nonempty_audio_without_timeline_metadata_cannot_publish_complete() {
    let dir = tempfile_dir("nonempty-audio-without-metadata");
    let session = SessionId::new("s-nonempty-audio-without-metadata").unwrap();
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    recording
        .audio
        .as_mut()
        .unwrap()
        .write_all(&[1, 0])
        .unwrap();
    recording.data_bytes = 2;

    let result = recording.finalize().unwrap();

    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(scan_recordings(&dir).unwrap().complete.is_empty());
}

#[test]
fn every_commit_fault_leaves_an_explicit_partial_candidate_not_a_complete_session() {
    for point in CommitFaultPoint::ALL {
        let dir = tempfile_dir(&format!("fault-{point:?}"));
        let session = SessionId::new("s-fault").unwrap();
        let mut recording = StreamingRecording::create_with_fault(&dir, session, point).unwrap();
        if point == CommitFaultPoint::PeriodicFlush {
            recording.sync_interval_samples = 1;
        }
        let _ = recording.append_pcm16(&[1, 0, 2, 0]);
        let _ = recording.finalize();

        let scanned = scan_recordings(&dir).unwrap();
        assert!(
            scanned.complete.is_empty(),
            "{point:?} published a complete recording"
        );
        assert_eq!(
            scanned.partial.len(),
            1,
            "{point:?} hid the partial recovery candidate"
        );
    }
}

#[test]
fn partial_finalization_publishes_a_hashed_partial_capture_lineage() {
    let dir = tempfile_dir("partial-lineage");
    let session = SessionId::new("s-partial-lineage").unwrap();
    let mut recording =
        StreamingRecording::create_with_fault(&dir, session.clone(), CommitFaultPoint::AudioSync)
            .unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();

    let result = recording.finalize().unwrap();

    let lineage = result.partial_lineage.expect("partial capture lineage");
    assert_eq!(result.status, CaptureStatus::Partial);
    assert_eq!(
        lineage.capture_sidecar_sha256,
        sha256_file(&dir.join(&lineage.capture_sidecar_file)).unwrap()
    );
    let sidecar: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join(&lineage.capture_sidecar_file)).unwrap())
            .unwrap();
    assert_eq!(sidecar["sessionId"], session.as_str());
    assert_eq!(sidecar["status"], "partial");
    let scanned = scan_recordings(&dir).unwrap();
    assert!(scanned.complete.is_empty());
    assert_eq!(scanned.partial.len(), 1);
}

#[test]
fn cached_finalization_receipts_allow_sidecar_replacement_and_revalidate_identity() {
    let dir = tempfile_dir("receipt-handle-lifetime");
    let session = SessionId::new("s-receipt-handle-lifetime").unwrap();
    let paths = RecordingPaths::new(&dir, session.clone());
    let mut recording = StreamingRecording::create(&dir, session).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();

    let first = recording.finalize().unwrap();
    let second = recording.finalize().unwrap();

    first.revalidate_capture_sidecar().unwrap();
    second.revalidate_capture_sidecar().unwrap();
    let displaced = paths.sidecar.with_extension("displaced");
    fs::rename(&paths.sidecar, &displaced).unwrap();
    fs::write(&paths.sidecar, b"replacement sidecar").unwrap();
    assert!(displaced.is_file());
    assert!(paths.sidecar.is_file());
    assert!(first.revalidate_capture_sidecar().is_err());
    assert!(recording
        .finalize()
        .unwrap()
        .revalidate_capture_sidecar()
        .is_err());
}

#[test]
fn scanner_reports_damaged_complete_commit_with_residual_private_artifacts() {
    let dir = tempfile_dir("damaged-commit-scan");
    let session = SessionId::new("s-damaged-commit-scan").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    recording.finalize().unwrap();
    fs::write(
        dir.join(format!("live-{session}.capture.journal.part")),
        b"residual journal",
    )
    .unwrap();
    fs::write(
        dir.join(format!("live-{session}.commit.json")),
        b"{not json",
    )
    .unwrap();

    let scan = scan_recordings(&dir).unwrap();

    assert!(scan.complete.is_empty());
    assert!(scan.partial.is_empty());
    assert_eq!(scan.damaged.len(), 1);
    assert_eq!(scan.damaged[0].session_id, session);
    assert!(scan.damaged[0].reason.contains("parse"));
    fs::remove_dir_all(dir).ok();
}
