use super::*;

#[test]
fn read_text_file_rejects_non_transcripts() {
    assert!(read_text_file_at("recording.mp3".into()).is_err());
}

#[test]
fn read_text_preview_rejects_uncommitted_live_transcript() {
    let dir = temp_test_dir("preview-cap");
    let transcript = dir.join("live-100.txt");
    std::fs::write(&transcript, "abcdef").unwrap();

    assert!(read_text_preview_at_from_dir(transcript.display().to_string(), 3, &dir).is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn canonical_transcript_read_and_preview_consume_the_validated_handle() {
    let dir = temp_test_dir("validated-transcript-handle");
    let session = SessionId::new("s-validated-transcript-handle").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording.append_pcm16(&[1, 0]).unwrap();
    let capture = recording.finalize().unwrap();
    crate::live::recordings::save_finalized_capture_to_dir_for_test(&dir, "verified text", capture)
        .unwrap();
    let transcript = dir.join(format!("live-{session}.txt"));

    assert_eq!(
        read_text_file_at_from_dir(transcript.display().to_string(), &dir).unwrap(),
        "verified text\n"
    );
    assert_eq!(
        read_text_preview_at_from_dir(transcript.display().to_string(), 8, &dir).unwrap(),
        "verified"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn read_text_preview_rejects_uncommitted_multibyte_transcript() {
    let dir = temp_test_dir("preview-multibyte");
    let transcript = dir.join("live-105.txt");
    std::fs::write(&transcript, "abcdefg€").unwrap();

    assert!(read_text_preview_at_from_dir(transcript.display().to_string(), 1, &dir).is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_read_rejects_directory_after_canonicalization() {
    let dir = temp_test_dir("txt-dir");
    let transcript_dir = dir.join("live-101.txt");
    std::fs::create_dir_all(&transcript_dir).unwrap();

    let error = read_text_file_at_from_dir(transcript_dir.display().to_string(), &dir).unwrap_err();

    assert_eq!(error, "Only transcript text files can be read.");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn read_text_file_rejects_uncommitted_oversized_transcripts() {
    let dir = temp_test_dir("oversized-read");
    let transcript = dir.join("live-102.txt");
    std::fs::write(
        &transcript,
        vec![b'a'; (MAX_TRANSCRIPT_READ_BYTES as usize) + 1],
    )
    .unwrap();

    let error = read_text_file_at_from_dir(transcript.display().to_string(), &dir).unwrap_err();

    assert_eq!(
        error,
        "Only Yap-owned canonical live transcripts can be read."
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn transcript_reads_reject_external_text_files() {
    let owned_dir = temp_test_dir("owned-live-read");
    let external_dir = temp_test_dir("external-transcript-read");
    let transcript = external_dir.join("live-103.txt");
    std::fs::write(&transcript, "secret").unwrap();

    assert_eq!(
        read_text_file_at_from_dir(transcript.display().to_string(), &owned_dir).unwrap_err(),
        "Only Yap-owned canonical live transcripts can be read."
    );
    assert_eq!(
        read_text_preview_at_from_dir(transcript.display().to_string(), 10, &owned_dir)
            .unwrap_err(),
        "Only Yap-owned canonical live transcripts can be read."
    );
    assert_eq!(
        write_polished_text_at_from_dir(
            transcript.display().to_string(),
            "safe".into(),
            &owned_dir,
        )
        .unwrap_err(),
        "Only Yap-owned canonical live transcripts can be polished."
    );
    std::fs::remove_dir_all(owned_dir).ok();
    std::fs::remove_dir_all(external_dir).ok();
}

#[test]
fn transcript_actions_reject_resolved_non_transcript_files() {
    let dir = temp_test_dir("txt-symlink");
    let target_dir = dir.join("reparse-target");
    std::fs::create_dir_all(&target_dir).unwrap();
    let target = target_dir.join("secret.json");
    let link = dir.join("live-104.txt");
    std::fs::write(&target, "{}").unwrap();
    create_reparse_point(&target, &link).expect(
        "reparse fixture creation failed; tests require file symlinks or NTFS directory junctions",
    );
    let link_metadata = std::fs::symlink_metadata(&link).unwrap();
    assert!(
        link_metadata.file_type().is_symlink() || metadata_is_reparse_point(&link_metadata),
        "fixture must be a symlink or Windows reparse point"
    );

    assert_eq!(
        read_text_file_at_from_dir(link.display().to_string(), &dir).unwrap_err(),
        "Only transcript text files can be read."
    );
    assert_eq!(
        read_text_preview_at_from_dir(link.display().to_string(), 10, &dir).unwrap_err(),
        "Only transcript text files can be read."
    );
    assert_eq!(
        write_polished_text_at_from_dir(link.display().to_string(), "safe".into(), &dir)
            .unwrap_err(),
        "Only transcript text files can be polished."
    );
    remove_reparse_point(&link).unwrap();
    std::fs::remove_dir_all(dir).ok();
}
