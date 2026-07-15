use super::*;

#[test]
fn transcript_text_prefers_final_then_partial() {
    let mut view = live_view(Some("final"), Some("partial"));

    assert_eq!(transcript_text(&view).as_deref(), Some("final"));
    view.final_text = None;
    assert_eq!(transcript_text(&view).as_deref(), Some("partial"));
}

#[test]
fn completed_transcript_text_never_promotes_a_partial() {
    let mut view = live_view(None, Some("partial"));
    assert_eq!(completed_transcript_text(&view), None);

    view.final_text = Some("final".into());
    assert_eq!(completed_transcript_text(&view).as_deref(), Some("final"));
}

#[test]
fn transcript_text_cleans_streaming_artifacts() {
    let mut view = live_view(Some("  THank   you.. "), None);

    assert_eq!(transcript_text(&view).as_deref(), Some("Thank you."));
    view.final_text = Some("NASA called.".into());
    assert_eq!(transcript_text(&view).as_deref(), Some("NASA called."));
}

#[test]
fn transcript_revision_rejects_a_linked_prior_revision_when_supported() {
    let dir = test_dir("linked-transcript-revision");
    let session = SessionId::new("s-linked-transcript-revision").unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));
    let transcript_receipt = write_new_text_file(&transcript, "first\n").unwrap();
    write_transcript_revision(
        &dir,
        &session,
        &"a".repeat(64),
        &transcript_receipt,
        "first",
        ResultStatus::Complete,
    )
    .unwrap();
    let outside =
        std::env::temp_dir().join(format!("yap-linked-revision-target-{}", std::process::id()));
    std::fs::remove_file(&outside).ok();
    std::fs::write(&outside, "outside revision\n").unwrap();
    let first = transcript_revision_path(&dir, &session, 1);
    std::fs::remove_file(&first).unwrap();
    if let Err(error) = create_file_symlink_for_test(&outside, &first) {
        skip_link_test_or_panic(error);
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(dir).ok();
        return;
    }

    assert!(write_transcript_revision(
        &dir,
        &session,
        &"a".repeat(64),
        &transcript_receipt,
        "second",
        ResultStatus::Complete,
    )
    .is_err());
    assert!(!transcript_revision_path(&dir, &session, 2).exists());
    std::fs::remove_file(&first).ok();
    std::fs::remove_file(&outside).ok();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn partial_capture_before_sidecar_publication_keeps_transcript_and_publishes_partial_revision() {
    assert_partial_capture_transcript(CommitFaultPoint::AudioSync);
}

#[test]
fn partial_capture_after_sidecar_publication_keeps_transcript_and_publishes_partial_revision() {
    assert_partial_capture_transcript(CommitFaultPoint::CommitSync);
}

#[test]
fn worker_panic_still_publishes_a_usable_transcript_without_fabricating_history() {
    assert_unavailable_recording_transcript("s-worker-panic", true);
}

#[test]
fn unavailable_worker_still_publishes_a_usable_transcript_without_fabricating_history() {
    assert_unavailable_recording_transcript("s-worker-unavailable", false);
}

#[test]
fn transcript_sync_failure_does_not_rename_the_partial_file() {
    let dir = test_dir("transcript-sync-failure");
    let transcript = dir.join("live-301.txt");
    let renamed = std::cell::Cell::new(false);

    let error = write_new_text_file_with(
        &transcript,
        "hello\n",
        |_| Err(std::io::Error::other("injected transcript sync failure")),
        |_, _, _| {
            renamed.set(true);
            Err("test publisher should not be called".into())
        },
    )
    .unwrap_err();

    assert!(error.contains("injected transcript sync failure"));
    assert!(!renamed.get());
    assert!(!transcript.exists());
    assert!(!partial_text_path(&transcript).unwrap().exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_pre_link_replacement_keeps_the_attacker_staging_file_and_writes_no_revision() {
    let dir = test_dir("transcript-pre-link-replacement");
    let session = SessionId::new("s-transcript-pre-link-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));
    let partial = partial_text_path(&transcript).unwrap();

    let error = save_finalized_capture_to_dir_with_text_publisher(
        &dir,
        &live_view(Some("owned transcript"), None),
        Some(capture),
        |source, destination, owned| {
            let displaced = source.with_extension("displaced");
            std::fs::rename(source, &displaced).map_err(|error| error.to_string())?;
            std::fs::write(source, b"attacker staging").map_err(|error| error.to_string())?;
            recording::publish_no_replace(source, destination, owned, "publish live transcript")
        },
    )
    .unwrap_err();

    assert!(error.contains("staging path no longer names the owned file"));
    assert_eq!(std::fs::read(&partial).unwrap(), b"attacker staging");
    assert!(!transcript.exists());
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert_eq!(recording::scan_recordings(&dir).unwrap().complete.len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_post_link_replacement_keeps_the_attacker_text_and_writes_no_revision() {
    let dir = test_dir("transcript-post-link-replacement");
    let session = SessionId::new("s-transcript-post-link-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));

    let error = save_finalized_capture_to_dir_with_text_publisher(
        &dir,
        &live_view(Some("owned transcript"), None),
        Some(capture),
        |source, destination, owned| {
            recording::publish_no_replace_with_after_link_for_test(
                source,
                destination,
                owned,
                "publish live transcript",
                || {
                    let displaced = destination.with_extension("displaced");
                    std::fs::rename(destination, displaced).unwrap();
                    std::fs::write(destination, b"attacker text").unwrap();
                },
            )
        },
    )
    .unwrap_err();

    assert!(error.contains("published destination does not name the owned file"));
    assert_eq!(std::fs::read(&transcript).unwrap(), b"attacker text");
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert_eq!(recording::scan_recordings(&dir).unwrap().complete.len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_replacement_after_publication_preserves_independent_text_without_a_revision() {
    let dir = test_dir("transcript-post-publication-replacement");
    let session = SessionId::new("s-transcript-post-publication-replacement").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));

    let saved = save_finalized_capture_to_dir_with_text_publisher(
        &dir,
        &live_view(Some("owned transcript"), None),
        Some(capture),
        |source, destination, owned| {
            let published = recording::publish_no_replace(
                source,
                destination,
                owned,
                "publish live transcript",
            )?;
            let displaced = destination.with_extension("displaced");
            std::fs::rename(destination, displaced).map_err(|error| error.to_string())?;
            std::fs::write(destination, b"attacker transcript")
                .map_err(|error| error.to_string())?;
            Ok(published)
        },
    )
    .unwrap()
    .unwrap();

    assert_eq!(std::fs::read(&transcript).unwrap(), b"attacker transcript");
    assert!(saved
        .warning
        .as_deref()
        .unwrap_or_default()
        .contains("Transcript revision was not saved"));
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    let scan = recording::scan_recordings(&dir).unwrap();
    assert_eq!(scan.complete.len(), 1);
    assert_eq!(scan.complete[0].manifest.session_id, session);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_receipt_allows_destination_move_and_revalidates_identity() {
    let dir = test_dir("transcript-receipt-handle-lifetime");
    let session = SessionId::new("s-transcript-receipt-handle-lifetime").unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));

    let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

    receipt.revalidate().unwrap();
    let displaced = transcript.with_extension("displaced");
    std::fs::rename(&transcript, &displaced).unwrap();
    std::fs::write(&transcript, "replacement transcript\n").unwrap();
    assert!(displaced.is_file());
    assert!(transcript.is_file());
    assert!(receipt.revalidate().is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_replacement_before_revision_publication_writes_no_revision() {
    let dir = test_dir("transcript-revision-pre-publication-replacement");
    let session = SessionId::new("s-transcript-revision-pre-publication").unwrap();
    let mut recording_capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording_capture.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording_capture
        .finalize()
        .unwrap()
        .committed
        .unwrap()
        .manifest;
    let transcript = dir.join(format!("live-{session}.txt"));
    let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

    let error = write_transcript_revision_with_barrier(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "owned transcript",
        ResultStatus::Complete,
        |barrier| {
            if barrier == TranscriptRevisionPublicationBarrier::BeforePublication {
                let displaced = transcript.with_extension("displaced");
                std::fs::rename(&transcript, displaced).unwrap();
                std::fs::write(&transcript, "replacement transcript\n").unwrap();
            }
        },
    )
    .unwrap_err();

    assert!(error.contains("transcript path no longer names"));
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert_eq!(
        std::fs::read_to_string(&transcript).unwrap(),
        "replacement transcript\n"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_replacement_after_revision_publication_is_not_selected_by_history() {
    let dir = test_dir("transcript-revision-post-publication-replacement");
    let session = SessionId::new("s-transcript-revision-post-publication").unwrap();
    let mut recording_capture = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording_capture.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording_capture
        .finalize()
        .unwrap()
        .committed
        .unwrap()
        .manifest;
    let transcript = dir.join(format!("live-{session}.txt"));
    let receipt = write_new_text_file(&transcript, "owned transcript\n").unwrap();

    let error = write_transcript_revision_with_barrier(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "owned transcript",
        ResultStatus::Complete,
        |barrier| {
            if barrier == TranscriptRevisionPublicationBarrier::AfterPublication {
                let displaced = transcript.with_extension("displaced");
                std::fs::rename(&transcript, displaced).unwrap();
                std::fs::write(&transcript, "replacement transcript\n").unwrap();
            }
        },
    )
    .unwrap_err();

    assert!(error.contains("transcript path no longer names"));
    assert!(transcript_revision_path(&dir, &session, 1).is_file());
    assert!(!has_valid_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
    ));
    let sessions = list_session_files_from_dir(&dir).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].output_path, sessions[0].source_path);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn replaced_capture_sidecar_preserves_text_but_blocks_transcript_revision() {
    let dir = test_dir("transcript-sidecar-revalidation");
    let session = SessionId::new("s-transcript-sidecar-revalidation").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    let sidecar = dir.join(format!("live-{session}.capture.json"));
    let displaced = sidecar.with_extension("displaced");
    std::fs::rename(&sidecar, displaced).unwrap();
    std::fs::write(&sidecar, b"attacker sidecar").unwrap();

    let saved =
        save_finalized_capture_to_dir(&dir, &live_view(Some("survives"), None), Some(capture))
            .unwrap()
            .unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join(format!("live-{session}.txt"))).unwrap(),
        "survives\n"
    );
    assert!(saved
        .warning
        .unwrap()
        .contains("Transcript revision was not saved"));
    assert!(!transcript_revision_path(&dir, &session, 1).exists());
    assert!(recording::scan_recordings(&dir)
        .unwrap()
        .complete
        .is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_revisions_are_create_new_and_monotonic() {
    let dir = test_dir("transcript-revisions");
    let session = SessionId::new("s-revisions").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording.finalize().unwrap().committed.unwrap().manifest;
    let text_path = dir.join(format!("live-{session}.txt"));
    let transcript_receipt = write_new_text_file(&text_path, "first\n").unwrap();

    write_transcript_revision(
        &dir,
        &manifest.session_id,
        &manifest.capture_sidecar_sha256,
        &transcript_receipt,
        "first",
        ResultStatus::Complete,
    )
    .unwrap();
    write_transcript_revision(
        &dir,
        &manifest.session_id,
        &manifest.capture_sidecar_sha256,
        &transcript_receipt,
        "second",
        ResultStatus::Complete,
    )
    .unwrap();

    assert!(transcript_revision_path(&dir, &session, 1).is_file());
    assert!(transcript_revision_path(&dir, &session, 2).is_file());
    let revision = std::fs::read_to_string(transcript_revision_path(&dir, &session, 1)).unwrap();
    let revision: serde_json::Value = serde_json::from_str(&revision).unwrap();
    assert_eq!(revision["textFile"], format!("live-{session}.txt"));
    assert_eq!(revision["textSha256"], transcript_receipt.sha256());
    assert_eq!(revision["modelId"], crate::stt::nemotron::MODEL_ID);
    let sessions = list_session_files_from_dir(&dir).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].output_path, text_path.display().to_string());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn highest_corrupt_revision_does_not_fall_back_to_a_valid_lower_revision() {
    let dir = test_dir("highest-corrupt-revision");
    let session = SessionId::new("s-highest-corrupt-revision").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let manifest = recording.finalize().unwrap().committed.unwrap().manifest;
    let transcript = dir.join(format!("live-{session}.txt"));
    let receipt = write_new_text_file(&transcript, "first\n").unwrap();
    write_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "first",
        ResultStatus::Complete,
    )
    .unwrap();
    write_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
        &receipt,
        "second",
        ResultStatus::Complete,
    )
    .unwrap();
    std::fs::write(transcript_revision_path(&dir, &session, 2), "tampered").unwrap();

    assert!(!has_valid_transcript_revision(
        &dir,
        &session,
        &manifest.capture_sidecar_sha256,
    ));
    let saved = list_session_files_from_dir(&dir).unwrap().pop().unwrap();
    assert_eq!(saved.output_path, saved.source_path);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn write_new_text_file_does_not_scan_partial_transcripts() {
    let dir = std::env::temp_dir().join(format!("yap-live-text-partial-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join("live-77.txt");
    let partial = partial_text_path(&transcript).unwrap();
    std::fs::write(&partial, "stale").unwrap();

    let sessions = list_session_files_from_dir(&dir).unwrap();

    assert!(sessions.is_empty());
    std::fs::remove_dir_all(dir).ok();
}
