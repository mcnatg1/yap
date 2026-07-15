use super::*;

#[test]
fn playback_registry_restores_registered_recordings() {
    let dir = temp_test_dir("playback-registry");
    let registry = dir.join("registry.json");
    let media = dir.join("meeting.wav");
    std::fs::write(&media, b"RIFF").unwrap();

    let registered = register_playback_path_at(media.display().to_string(), &registry).unwrap();
    let restored = registered_playback_path_at(media.display().to_string(), &registry).unwrap();

    assert_eq!(restored, registered);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_admission_returns_revocable_url_and_exact_metadata() {
    let dir = temp_test_dir("playback-admission-metadata");
    let media = dir.join("meeting.wav");
    std::fs::write(&media, b"RIFFdata").unwrap();
    let owner = crate::media_protocol::MediaOwner::new();

    let admission = mint_playback_admission(&media.canonicalize().unwrap(), &owner).unwrap();

    assert!(admission.playback_path.starts_with("http://127.0.0.1:"));
    assert!(!admission.playback_path.contains("meeting.wav"));
    assert_eq!(admission.byte_length, "8");
    assert!(admission.waveform_eligible);
    assert!(owner.release(&admission.playback_path));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_restore_revalidates_native_authorization() {
    let dir = temp_test_dir("playback-admission-revalidate");
    let registry = dir.join("registry.json");
    let owned = dir.join("owned");
    let media = dir.join("meeting.wav");
    std::fs::create_dir_all(&owned).unwrap();
    std::fs::write(&media, b"RIFF").unwrap();
    register_playback_path_at_from_owned_dir(media.display().to_string(), &registry, &owned)
        .unwrap();
    std::fs::remove_file(&media).unwrap();

    let error =
        restore_playback_path_at(media.display().to_string(), &registry, &owned).unwrap_err();

    assert_eq!(error, "File no longer exists.");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_restore_ignores_pre_picker_general_registry_authority() {
    let dir = temp_test_dir("playback-pre-picker-authority");
    let legacy_registry = dir.join("recording-playback-registry.json");
    let native_job_registry = dir.join("recording-job-playback-registry.json");
    let owned = dir.join("owned");
    let media = dir.join("meeting.wav");
    std::fs::create_dir_all(&owned).unwrap();
    std::fs::write(&media, b"RIFF").unwrap();
    register_playback_path_at_from_owned_dir(media.display().to_string(), &legacy_registry, &owned)
        .unwrap();

    assert!(
        restore_playback_path_at(media.display().to_string(), &native_job_registry, &owned,)
            .is_err()
    );

    register_recording_job_playback_path_at_from_owned_dir(
        media.display().to_string(),
        &native_job_registry,
        &owned,
    )
    .unwrap();
    assert_eq!(
        restore_playback_path_at(media.display().to_string(), &native_job_registry, &owned,)
            .unwrap(),
        media.canonicalize().unwrap()
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn owned_playback_restore_keeps_one_canonical_path_during_catalog_validation() {
    let dir = temp_test_dir("owned-playback-stable-canonical-path");
    let registry = dir.join("registry.json");
    let owned = dir.join("owned");
    let nested = owned.join("nested");
    let media = owned.join("live-selected.wav");
    let requested = nested.join("..").join("live-selected.wav");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(&media, b"RIFF").unwrap();
    let canonical_media = media.canonicalize().unwrap();
    let mut validations = 0;

    let restored = restore_playback_path_at_with(
        requested.display().to_string(),
        &registry,
        &owned,
        |requested, requested_owned, require_transcript| {
            validations += 1;
            assert_eq!(requested_owned, owned);
            assert!(!require_transcript);
            assert_eq!(requested, canonical_media.as_path());
            Ok(requested.to_path_buf())
        },
    )
    .unwrap();

    assert_eq!(restored, canonical_media);
    assert_eq!(validations, 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_registry_rejects_unregistered_recordings() {
    let dir = temp_test_dir("playback-registry-unregistered");
    let registry = dir.join("registry.json");
    let media = dir.join("meeting.wav");
    std::fs::write(&media, b"RIFF").unwrap();

    let error = registered_playback_path_at(media.display().to_string(), &registry).unwrap_err();

    assert_eq!(error, "Recording file is not registered for playback.");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_registry_recovers_from_invalid_json() {
    let dir = temp_test_dir("playback-registry-invalid");
    let registry = dir.join("registry.json");
    let media = dir.join("meeting.wav");
    std::fs::write(&registry, "not-json").unwrap();
    std::fs::write(&media, b"RIFF").unwrap();

    register_playback_path_at(media.display().to_string(), &registry).unwrap();
    let restored = registered_playback_path_at(media.display().to_string(), &registry).unwrap();

    assert_eq!(restored.file_name().unwrap(), "meeting.wav");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_registry_rejects_unsupported_versions() {
    let dir = temp_test_dir("playback-registry-version");
    let registry = dir.join("registry.json");
    let media = dir.join("meeting.wav");
    std::fs::write(&registry, r#"{"version":99,"paths":[]}"#).unwrap();
    std::fs::write(&media, b"RIFF").unwrap();

    let error = register_playback_path_at(media.display().to_string(), &registry).unwrap_err();

    assert!(error.contains("Unsupported playback registry version"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pre_native_picker_registry_cannot_restore_playback_authority() {
    let dir = temp_test_dir("playback-registry-pre-native-picker");
    let registry = dir.join("registry.json");
    let media = dir.join("meeting.wav");
    std::fs::write(&media, b"RIFF").unwrap();
    std::fs::write(
        &registry,
        format!(
            r#"{{"version":1,"paths":[{}]}}"#,
            serde_json::to_string(&media.display().to_string()).unwrap()
        ),
    )
    .unwrap();

    let error = registered_playback_path_at(media.display().to_string(), &registry).unwrap_err();
    assert_eq!(error, "Recording file is not registered for playback.");

    register_playback_path_at(media.display().to_string(), &registry).unwrap();
    let rewritten: RecordingPlaybackRegistry =
        serde_json::from_str(&std::fs::read_to_string(&registry).unwrap()).unwrap();
    assert_eq!(rewritten.version, NATIVE_SELECTION_REGISTRY_VERSION);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_registry_does_not_evict_trusted_paths_at_capacity() {
    let dir = temp_test_dir("playback-registry-capacity");
    let registry = dir.join("registry.json");
    let media = dir.join("new.wav");
    let paths = (0..MAX_REGISTERED_PLAYBACK_PATHS)
        .map(|index| dir.join(format!("registered-{index}.wav")))
        .collect::<Vec<_>>();
    write_registered_playback_paths(&registry, &paths).unwrap();
    std::fs::write(&media, b"RIFF").unwrap();

    let error = register_playback_path_at(media.display().to_string(), &registry).unwrap_err();

    assert!(error.contains("playback registry is full"));
    assert_eq!(
        read_registered_playback_paths(&registry).unwrap().len(),
        paths.len()
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn playback_registry_serializes_concurrent_registrations() {
    let dir = temp_test_dir("playback-registry-concurrent");
    let registry = dir.join("registry.json");
    let paths = (0..20)
        .map(|index| {
            let path = dir.join(format!("meeting-{index}.wav"));
            std::fs::write(&path, b"RIFF").unwrap();
            path
        })
        .collect::<Vec<_>>();

    let threads = paths
        .iter()
        .cloned()
        .map(|path| {
            let registry = registry.clone();
            std::thread::spawn(move || {
                register_playback_path_at(path.display().to_string(), &registry)
            })
        })
        .collect::<Vec<_>>();

    for thread in threads {
        thread.join().unwrap().unwrap();
    }
    assert_eq!(read_registered_playback_paths(&registry).unwrap().len(), 20);
    std::fs::remove_dir_all(dir).ok();
}
