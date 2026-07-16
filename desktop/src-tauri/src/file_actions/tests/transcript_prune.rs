use super::*;

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
