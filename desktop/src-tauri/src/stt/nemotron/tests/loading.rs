use super::*;

#[cfg(windows)]
#[test]
fn load_verification_hashes_current_artifacts_even_when_markers_claim_ready() {
    let dir = TestDir::new();
    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }
    let artifact = &TEST_ARTIFACTS[0];
    let path = dir.path().join(artifact.file);
    std::fs::write(&path, b"xyz").unwrap();
    std::fs::write(
        path.with_extension("verified"),
        format!("{}\n{}\n", artifact.sha256, artifact.bytes),
    )
    .unwrap();
    assert_eq!(marker_state(&path, artifact), MarkerState::Valid);

    let loader_called = std::cell::Cell::new(false);
    let result = load_local_fallback_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS, |_| {
        loader_called.set(true);
        Ok(())
    });

    assert!(matches!(result, Err(SttError::ModelCorrupt)));
    assert!(!loader_called.get());
}

#[cfg(windows)]
#[test]
fn native_load_uses_a_verified_snapshot_when_the_installed_model_changes() {
    let dir = TestDir::new();
    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }
    let original = dir.path().join(TEST_ARTIFACTS[0].file);
    let mut snapshot = None;

    let loaded = load_local_fallback_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS, |paths| {
        snapshot = Some(paths.encoder.clone());
        assert_ne!(paths.encoder, original);
        std::fs::write(&original, b"xyz").unwrap();
        assert_eq!(
            std::fs::read(&paths.encoder).unwrap(),
            TEST_ARTIFACT_CONTENTS
        );
        Ok(())
    })
    .unwrap();

    let snapshot_root = snapshot.unwrap().parent().unwrap().to_path_buf();
    assert_eq!(std::fs::read(&original).unwrap(), b"xyz");
    assert!(snapshot_root.exists());
    drop(loaded);
    assert!(!snapshot_root.exists());
}

#[cfg(windows)]
#[test]
fn load_lease_keeps_snapshot_artifacts_immutable_until_the_engine_retires() {
    let dir = TestDir::new();
    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }
    let original = dir.path().join(TEST_ARTIFACTS[0].file);
    let mut snapshot = None;
    let loaded = load_local_fallback_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS, |paths| {
        snapshot = Some(paths.encoder.clone());
        Ok(())
    })
    .unwrap();
    let snapshot = snapshot.unwrap();
    let snapshot_root = snapshot.parent().unwrap().to_path_buf();

    assert!(std::fs::OpenOptions::new()
        .write(true)
        .open(&snapshot)
        .is_err());
    assert!(std::fs::OpenOptions::new()
        .write(true)
        .open(&original)
        .is_ok());
    drop(loaded);
    assert!(!snapshot_root.exists());
}

#[cfg(not(windows))]
#[test]
fn native_model_loading_fails_closed_off_the_supported_windows_target() {
    let dir = TestDir::new();
    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(dir.path(), artifact);
    }
    let loader_called = std::cell::Cell::new(false);

    let result = load_local_fallback_at_with_artifacts(dir.path(), true, TEST_ARTIFACTS, |_| {
        loader_called.set(true);
        Ok(())
    });

    assert!(matches!(result, Err(SttError::ModelCorrupt)));
    assert!(!loader_called.get());
}

#[cfg(windows)]
#[test]
fn load_verification_rejects_a_junction_model_root() {
    let dir = TestDir::new();
    let real_root = dir.path().join("real-model");
    let linked_root = dir.path().join("linked-model");
    std::fs::create_dir(&real_root).unwrap();
    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(&real_root, artifact);
    }
    create_junction(&real_root, &linked_root).unwrap();

    let result =
        load_local_fallback_at_with_artifacts(&linked_root, true, TEST_ARTIFACTS, |_| Ok(()));

    assert!(matches!(result, Err(SttError::ModelCorrupt)));
    std::fs::remove_dir(&linked_root).unwrap();
}

#[cfg(windows)]
#[test]
fn load_verification_rejects_a_junction_in_the_model_root_chain() {
    let dir = TestDir::new();
    let real_parent = dir.path().join("real-parent");
    let real_root = real_parent.join("model");
    let linked_parent = dir.path().join("linked-parent");
    std::fs::create_dir_all(&real_root).unwrap();
    for artifact in TEST_ARTIFACTS {
        write_verified_artifact(&real_root, artifact);
    }
    create_junction(&real_parent, &linked_parent).unwrap();

    let result = load_local_fallback_at_with_artifacts(
        &linked_parent.join("model"),
        true,
        TEST_ARTIFACTS,
        |_| Ok(()),
    );

    assert!(matches!(result, Err(SttError::ModelCorrupt)));
    std::fs::remove_dir(&linked_parent).unwrap();
}

#[cfg(windows)]
#[test]
fn stale_load_snapshot_cleanup_removes_only_owned_directories() {
    let dir = TestDir::new();
    let stale = dir.path().join(".yap-model-load-stale");
    let unrelated = dir.path().join("keep-me");
    std::fs::create_dir(&stale).unwrap();
    std::fs::write(stale.join("partial.bin"), b"partial").unwrap();
    std::fs::create_dir(&unrelated).unwrap();

    load_guard::cleanup_stale_snapshots(dir.path()).unwrap();

    assert!(!stale.exists());
    assert!(unrelated.exists());
}

#[cfg(windows)]
#[test]
fn stale_load_snapshot_cleanup_never_follows_a_junction() {
    let dir = TestDir::new();
    let target = dir.path().join("outside-snapshot");
    let linked_snapshot = dir.path().join(".yap-model-load-linked");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.bin"), b"keep").unwrap();
    create_junction(&target, &linked_snapshot).unwrap();

    assert_eq!(
        load_guard::cleanup_stale_snapshots(dir.path()),
        Err(SttError::ModelCorrupt)
    );
    assert_eq!(std::fs::read(target.join("keep.bin")).unwrap(), b"keep");
    std::fs::remove_dir(&linked_snapshot).unwrap();
}

#[cfg(windows)]
fn create_junction(target: &Path, link: &Path) -> std::io::Result<()> {
    let output = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}
