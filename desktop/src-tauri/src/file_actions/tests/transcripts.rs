use super::*;

#[test]
fn bounded_transcript_io_keeps_capacity_owned_until_timed_out_work_finishes() {
    let limiter = Arc::new(Semaphore::new(1));
    let first = tauri::async_runtime::block_on(run_bounded_transcript_io(
        Arc::clone(&limiter),
        Duration::from_millis(10),
        "Test read",
        || {
            std::thread::sleep(Duration::from_millis(100));
            Ok("late".into())
        },
    ));
    assert!(first.unwrap_err().contains("timed out"));
    assert_eq!(limiter.available_permits(), 0);

    let second_ran = Arc::new(AtomicBool::new(false));
    let second_ran_in_work = Arc::clone(&second_ran);
    let second = tauri::async_runtime::block_on(run_bounded_transcript_io(
        Arc::clone(&limiter),
        Duration::from_millis(10),
        "Test read",
        move || {
            second_ran_in_work.store(true, Ordering::SeqCst);
            Ok("unexpected".into())
        },
    ));
    assert!(second.unwrap_err().contains("filesystem capacity"));
    assert!(!second_ran.load(Ordering::SeqCst));

    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while limiter.available_permits() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(limiter.available_permits(), 1);
}

#[test]
fn bounded_transcript_io_returns_successful_work() {
    let result = tauri::async_runtime::block_on(run_bounded_transcript_io(
        Arc::new(Semaphore::new(1)),
        Duration::from_secs(1),
        "Test read",
        || Ok("ready".into()),
    ));

    assert_eq!(result.unwrap(), "ready");
}

#[test]
fn read_text_file_rejects_non_transcripts() {
    assert!(read_text_file_at("recording.mp3".into()).is_err());
}

#[test]
fn hidden_prune_authorizes_only_missing_primary_owned_transcripts() {
    let dir = temp_test_dir("hidden-prune-owned");
    let existing = dir.join("live-s-100.txt");
    let missing = dir.join("live-s-101.txt");
    std::fs::write(&existing, "still here").unwrap();
    let canonical_dir = dir.canonicalize().unwrap();

    let resolutions = resolve_owned_live_transcript_paths_from_dir(
        vec![
            existing.display().to_string(),
            missing.display().to_string(),
        ],
        &dir,
    )
    .unwrap();

    assert_eq!(resolutions.len(), 2);
    assert_eq!(
        resolutions[0].requested_path,
        existing.display().to_string()
    );
    assert_eq!(
        resolutions[0].canonical_path.as_deref(),
        Some(
            crate::live::recordings::stable_existing_path_string(
                &canonical_dir.join("live-s-100.txt"),
            )
            .as_str(),
        )
    );
    assert!(!resolutions[0].missing);
    assert_eq!(
        resolutions[1],
        OwnedLiveTranscriptPathResolution {
            requested_path: missing.display().to_string(),
            canonical_path: Some(crate::live::recordings::stable_existing_path_string(
                &canonical_dir.join("live-s-101.txt")
            )),
            missing: true,
        }
    );
    std::fs::remove_dir_all(dir).ok();
}

#[cfg(target_os = "windows")]
#[test]
fn hidden_prune_resolves_legacy_case_alias_to_canonical_output() {
    let dir = temp_test_dir("hidden-prune-case-alias");
    let transcript = dir.join("live-s-108.txt");
    std::fs::write(&transcript, "still here").unwrap();
    let canonical_dir = dir.canonicalize().unwrap();
    let requested = dir
        .display()
        .to_string()
        .to_uppercase()
        .replace("LIVE-RECORDINGS", "live-recordings");
    let requested = std::path::PathBuf::from(requested).join("live-s-108.txt");

    let resolutions =
        resolve_owned_live_transcript_paths_from_dir(vec![requested.display().to_string()], &dir)
            .unwrap();

    assert_eq!(resolutions.len(), 1);
    assert_eq!(
        resolutions[0].canonical_path.as_deref(),
        Some(
            crate::live::recordings::stable_existing_path_string(
                &canonical_dir.join("live-s-108.txt"),
            )
            .as_str(),
        )
    );
    assert!(!resolutions[0].missing);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn hidden_prune_rejects_untrusted_or_non_primary_paths() {
    let dir = temp_test_dir("hidden-prune-untrusted");
    let external = temp_test_dir("hidden-prune-external");
    let nested = dir.join("nested");
    std::fs::create_dir_all(&nested).unwrap();

    let confirmed = resolve_owned_live_transcript_paths_from_dir(
        vec![
            external.join("live-s-102.txt").display().to_string(),
            nested.join("live-s-103.txt").display().to_string(),
            "live-s-104.txt".into(),
            dir.join("live-105.polished.txt").display().to_string(),
            dir.join("live-nope.txt").display().to_string(),
            dir.join("notes.txt").display().to_string(),
        ],
        &dir,
    )
    .unwrap();

    assert!(confirmed.is_empty());
    std::fs::remove_dir_all(dir).ok();
    std::fs::remove_dir_all(external).ok();
}

#[test]
fn hidden_prune_preserves_existing_non_file_and_missing_root() {
    let dir = temp_test_dir("hidden-prune-directory");
    let directory = dir.join("live-106.txt");
    std::fs::create_dir_all(&directory).unwrap();
    let missing_root = dir.join("missing-root");

    assert!(resolve_owned_live_transcript_paths_from_dir(
        vec![directory.display().to_string()],
        &dir,
    )
    .unwrap()
    .is_empty());
    assert!(resolve_owned_live_transcript_paths_from_dir(
        vec![missing_root.join("live-107.txt").display().to_string()],
        &missing_root,
    )
    .unwrap()
    .is_empty());

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn hidden_prune_rejects_oversized_batches() {
    let dir = temp_test_dir("hidden-prune-bound");
    let candidates = (0..=MAX_HIDDEN_PRUNE_CANDIDATES)
        .map(|index| dir.join(format!("live-{index}.txt")).display().to_string())
        .collect();

    let error = resolve_owned_live_transcript_paths_from_dir(candidates, &dir).unwrap_err();

    assert!(error.contains("at most 200"));
    std::fs::remove_dir_all(dir).ok();
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

#[test]
fn polished_path_writes_sibling_file() {
    let path = polished_path(std::path::Path::new("C:/recordings/take.txt")).unwrap();
    assert_eq!(path.file_name().unwrap(), "take.polished.txt");
}

#[test]
fn atomic_text_write_replaces_stale_partial_file() {
    let dir = temp_test_dir("atomic-polish-write");
    let output = dir.join("take.polished.txt");
    let partial = dir.join("take.polished.txt.part");
    std::fs::write(&partial, "stale").unwrap();

    write_text_atomically(&output, "polished").unwrap();

    assert_eq!(std::fs::read_to_string(&output).unwrap(), "polished");
    assert!(!partial.exists());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn atomic_text_write_replaces_existing_output() {
    let dir = temp_test_dir("atomic-polish-overwrite");
    let output = dir.join("take.polished.txt");
    std::fs::write(&output, "old").unwrap();

    write_text_atomically(&output, "new").unwrap();

    assert_eq!(std::fs::read_to_string(&output).unwrap(), "new");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn atomic_text_write_uses_unique_temps_for_concurrent_writes() {
    let dir = temp_test_dir("atomic-polish-concurrent");
    let output = dir.join("take.polished.txt");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let left_output = output.clone();
    let left_barrier = std::sync::Arc::clone(&barrier);
    let left = std::thread::spawn(move || {
        left_barrier.wait();
        write_text_atomically(&left_output, "left")
    });
    let right_output = output.clone();
    let right_barrier = std::sync::Arc::clone(&barrier);
    let right = std::thread::spawn(move || {
        right_barrier.wait();
        write_text_atomically(&right_output, "right")
    });

    left.join().unwrap().unwrap();
    right.join().unwrap().unwrap();

    let text = std::fs::read_to_string(&output).unwrap();
    assert!(text == "left" || text == "right");
    let leftovers = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "part")
        })
        .count();
    assert_eq!(leftovers, 0);
    std::fs::remove_dir_all(dir).ok();
}
