use super::*;

#[test]
fn model_root_is_named_for_nemotron() {
    assert!(root_dir().ends_with(MODEL_DIR));
}

#[test]
fn pinned_artifacts_cover_sherpa_transducer_files() {
    let files = ARTIFACTS
        .iter()
        .map(|artifact| artifact.file)
        .collect::<Vec<_>>();
    assert_eq!(
        files,
        vec![
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "joiner.int8.onnx",
            "tokens.txt"
        ]
    );
    assert!(ARTIFACTS.iter().all(|artifact| artifact.sha256.len() == 64));
    assert_eq!(
        ARTIFACTS
            .iter()
            .map(|artifact| (artifact.file, artifact.bytes))
            .collect::<Vec<_>>(),
        vec![
            ("encoder.int8.onnx", 657_601_521),
            ("decoder.int8.onnx", 14_978_075),
            ("joiner.int8.onnx", 9_504_438),
            ("tokens.txt", 131_440),
        ]
    );
}

#[test]
fn marker_rejects_a_file_whose_size_does_not_match_the_pinned_artifact() {
    let dir = TestDir::new();
    let artifact = &TEST_ARTIFACTS[0];
    let path = dir.path().join(artifact.file);
    std::fs::write(&path, b"abcd").unwrap();
    std::fs::write(
        path.with_extension("verified"),
        format!("{}\n4\n", artifact.sha256),
    )
    .unwrap();

    assert_eq!(marker_state(&path, artifact), MarkerState::Stale);
}

#[test]
fn verification_rejects_length_before_accepting_a_matching_hash() {
    let dir = TestDir::new();
    let artifact = Artifact {
        file: "model.bin",
        sha256: TEST_ARTIFACT_SHA256,
        bytes: 4,
    };
    let path = dir.path().join(artifact.file);
    std::fs::write(&path, TEST_ARTIFACT_CONTENTS).unwrap();

    assert_eq!(
        verify_sha_and_mark(&path, &artifact),
        Err(SttError::ModelCorrupt)
    );
    assert!(!path.with_extension("verified").exists());
}

#[test]
fn model_verification_preflights_all_lengths_before_hashing() {
    let dir = TestDir::new();
    let artifacts = [
        Artifact {
            file: "encoder.int8.onnx",
            sha256: TEST_ARTIFACT_SHA256,
            bytes: 3,
        },
        Artifact {
            file: "decoder.int8.onnx",
            sha256: TEST_ARTIFACT_SHA256,
            bytes: 2,
        },
    ];
    let first = dir.path().join(artifacts[0].file);
    std::fs::write(&first, TEST_ARTIFACT_CONTENTS).unwrap();
    std::fs::write(dir.path().join(artifacts[1].file), b"x").unwrap();

    assert_eq!(
        verify_artifacts_at_with_progress(dir.path(), &artifacts, &mut |_| {}, || false),
        Err(SttError::ModelCorrupt)
    );
    assert!(!first.with_extension("verified").exists());
}

#[test]
fn model_status_projects_missing_ready_disabled_and_corrupted() {
    let dir = TestDir::new();

    assert_eq!(
        model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
        FallbackModelStatus::Missing
    );

    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }

    assert_eq!(
        model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
        FallbackModelStatus::Ready
    );
    assert_eq!(
        model_status_at_with_artifacts(dir.path(), false, TEST_ARTIFACTS).status,
        FallbackModelStatus::Disabled
    );

    let marker = dir
        .path()
        .join(TEST_ARTIFACTS[0].file)
        .with_extension("verified");
    std::fs::write(&marker, format!("{}\n999\n", TEST_ARTIFACTS[0].sha256)).unwrap();

    assert_eq!(
        model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
        FallbackModelStatus::Corrupted
    );
}

#[test]
fn synthetic_status_projection_covers_downloading_verifying_and_error() {
    let dir = TestDir::new();

    let downloading = status_view(
        dir.path(),
        FallbackModelStatus::Downloading,
        Some(32),
        Some(64),
        Some(50.0),
        Some(12.5),
        None,
    );
    assert_eq!(downloading.status, FallbackModelStatus::Downloading);
    assert_eq!(downloading.progress_percent, Some(50.0));
    assert_eq!(downloading.speed_mbps, Some(12.5));

    let verifying = status_view(
        dir.path(),
        FallbackModelStatus::Verifying,
        Some(64),
        Some(64),
        Some(100.0),
        None,
        Some("Verifying files".into()),
    );
    assert_eq!(verifying.status, FallbackModelStatus::Verifying);
    assert_eq!(verifying.message.as_deref(), Some("Verifying files"));

    let error = status_view(
        dir.path(),
        FallbackModelStatus::Error,
        None,
        None,
        None,
        None,
        Some("Download failed".into()),
    );
    assert_eq!(error.status, FallbackModelStatus::Error);
    assert_eq!(error.message.as_deref(), Some("Download failed"));
}

#[test]
fn resolve_model_preserves_corrupt_status() {
    let dir = TestDir::new();

    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }
    std::fs::remove_file(
        dir.path()
            .join(TEST_ARTIFACTS[0].file)
            .with_extension("verified"),
    )
    .unwrap();

    assert_eq!(
        classify_artifacts(dir.path(), TEST_ARTIFACTS),
        ArtifactInstallState::Corrupted
    );
    assert_eq!(
        resolve_model_at_with_artifacts(dir.path(), TEST_ARTIFACTS).unwrap_err(),
        SttError::ModelCorrupt
    );
}

#[test]
fn local_fallback_start_paths_require_enabled_even_when_ready() {
    let dir = TestDir::new();

    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }

    assert_eq!(
        local_fallback_start_paths_at_with_artifacts(dir.path(), false, TEST_ARTIFACTS)
            .unwrap_err(),
        SttError::FallbackDisabled
    );
    assert!(local_fallback_start_paths_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).is_ok());
}

#[test]
fn local_fallback_start_paths_preserve_missing_and_corrupt_failures() {
    let dir = TestDir::new();

    assert_eq!(
        local_fallback_start_paths_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).unwrap_err(),
        SttError::ModelMissing
    );

    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }
    std::fs::remove_file(
        dir.path()
            .join(TEST_ARTIFACTS[0].file)
            .with_extension("verified"),
    )
    .unwrap();

    assert_eq!(
        local_fallback_start_paths_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).unwrap_err(),
        SttError::ModelCorrupt
    );
}

#[test]
fn same_size_tampering_after_marker_creation_is_corrupted() {
    let dir = TestDir::new();

    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }
    tamper_artifact_same_size_after_marker(dir.path(), &TEST_ARTIFACTS[0]);

    assert_eq!(
        marker_state(&dir.path().join(TEST_ARTIFACTS[0].file), &TEST_ARTIFACTS[0]),
        MarkerState::Stale
    );
    assert_eq!(
        model_status_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS).status,
        FallbackModelStatus::Corrupted
    );
    assert_eq!(
        resolve_model_at_with_artifacts(dir.path(), TEST_ARTIFACTS).unwrap_err(),
        SttError::ModelCorrupt
    );
}
