use std::io::Write;

use super::super::persistence::{
    reconcile_publication_failure_with_parent_sync, reserve_unique_partial, snapshot_destination,
    write_atomically_with_before_publish,
};
use super::*;

#[test]
fn save_replaces_a_stale_partial_and_persists_only_the_public_schema() {
    let dir = temp_dir("atomic");
    let path = dir.join("server-settings.json");
    let partial = dir.join("server-settings.json.part");
    std::fs::write(
        &path,
        r#"{"schemaVersion":1,"enabled":false,"baseUrl":null}"#,
    )
    .unwrap();
    std::fs::write(&partial, "stale-secret").unwrap();
    let settings = ServerSettings {
        schema_version: CURRENT_SCHEMA_VERSION,
        enabled: true,
        base_url: Some("https://server.example/v1".into()),
    };

    let saved = save_to_path(&settings, &path, false).unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(saved.base_url.as_deref(), Some("https://server.example"));
    assert!(!partial.exists());
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["baseUrl", "enabled", "schemaVersion"]
    );
    assert_eq!(load_from_path(&path, false).unwrap(), saved);
    assert!(!std::fs::read_to_string(path).unwrap().contains("secret"));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn concurrent_writers_publish_only_complete_payloads_without_temp_leaks() {
    let dir = temp_dir("concurrent");
    let path = dir.join("server-settings.json");
    let initial = ServerSettings::default();
    save_to_path(&initial, &path, false).unwrap();

    let left = ServerSettings {
        schema_version: CURRENT_SCHEMA_VERSION,
        enabled: true,
        base_url: Some("https://left.example".into()),
    };
    let right = ServerSettings {
        schema_version: CURRENT_SCHEMA_VERSION,
        enabled: true,
        base_url: Some("https://right.example:8443/v1".into()),
    };
    let expected_right = ServerSettings {
        base_url: Some("https://right.example:8443".into()),
        ..right.clone()
    };
    let release = Arc::new(Barrier::new(3));
    let observing = Arc::new(AtomicBool::new(true));
    let observations = Arc::new(Mutex::new(Vec::new()));

    let observer_path = path.clone();
    let observer_running = Arc::clone(&observing);
    let observer_values = Arc::clone(&observations);
    let observer = std::thread::spawn(move || {
        while observer_running.load(Ordering::Acquire) {
            if let Ok(_lock) = acquire_settings_lock(&observer_path) {
                if let Ok(text) = std::fs::read_to_string(&observer_path) {
                    observer_values.lock().unwrap().push(text);
                }
            }
            std::thread::yield_now();
        }
    });

    let spawn_writer = |settings: ServerSettings| {
        let writer_path = path.clone();
        let writer_release = Arc::clone(&release);
        std::thread::spawn(move || {
            save_to_path_with_hooks(
                &settings,
                &writer_path,
                false,
                move || {
                    writer_release.wait();
                    Ok(())
                },
                || Ok(()),
                |_, _| Ok(()),
                |_| Ok(()),
            )
        })
    };
    let left_writer = spawn_writer(left.clone());
    let right_writer = spawn_writer(right.clone());
    release.wait();

    left_writer.join().unwrap().unwrap();
    right_writer.join().unwrap().unwrap();
    observing.store(false, Ordering::Release);
    observer.join().unwrap();

    let final_settings = load_from_path(&path, false).unwrap();
    assert!(final_settings == left || final_settings == expected_right);
    let observed = observations.lock().unwrap();
    assert!(!observed.is_empty());
    for text in observed.iter() {
        let settings: ServerSettings = serde_json::from_str(text).unwrap();
        assert!(settings == initial || settings == left || settings == expected_right);
    }
    assert_eq!(partial_files(&dir), Vec::<String>::new());

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn failed_publication_cleans_only_its_unique_partial() {
    let dir = temp_dir("failed-publication");
    let path = dir.join("server-settings.json");

    let error = write_atomically_with_before_publish(&path, b"{}", |_, _| {
        Err(std::io::Error::other("injected publication failure"))
    })
    .unwrap_err();

    assert!(error
        .to_string()
        .starts_with("Could not save server settings:"));
    assert!(error.to_string().contains("injected publication failure"));
    assert!(!path.exists());
    assert_eq!(partial_files(&dir), Vec::<String>::new());

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn failed_publication_with_unchanged_destination_is_prepublication_and_cleans_temp() {
    let dir = temp_dir("failed-unchanged-publication");
    let path = dir.join("server-settings.json");
    let original = b"original settings";
    std::fs::write(&path, original).unwrap();

    let error = write_atomically_with_before_publish(&path, b"intended settings", |_, _| {
        Err(std::io::Error::from_raw_os_error(1176))
    })
    .unwrap_err();

    assert!(matches!(error, ConfigError::SaveIo(_)));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_eq!(recovery_files(&dir), Vec::<PathBuf>::new());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_destination_after_failed_publish_is_indeterminate_and_preserves_recovery() {
    let dir = temp_dir("failed-missing-publication");
    let path = dir.join("server-settings.json");
    let intended = b"intended settings";
    std::fs::write(&path, b"original settings").unwrap();

    let error = write_atomically_with_before_publish(&path, intended, |_, destination| {
        std::fs::remove_file(destination)?;
        Err(std::io::Error::from_raw_os_error(1176))
    })
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::PublicationStateIndeterminate { .. }
    ));
    assert!(!path.exists());
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_one_recovery_with_contents(&dir, intended);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn moved_destination_after_failed_publish_is_indeterminate_and_preserves_recovery() {
    let dir = temp_dir("failed-moved-publication");
    let path = dir.join("server-settings.json");
    let moved = dir.join("server-settings.moved-by-api.json");
    let original = b"original settings";
    let intended = b"intended settings";
    std::fs::write(&path, original).unwrap();

    let error = write_atomically_with_before_publish(&path, intended, |_, destination| {
        std::fs::rename(destination, &moved)?;
        Err(std::io::Error::from_raw_os_error(1177))
    })
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::PublicationStateIndeterminate { .. }
    ));
    assert!(!path.exists());
    assert_eq!(std::fs::read(moved).unwrap(), original);
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_one_recovery_with_contents(&dir, intended);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn mutated_destination_after_failed_publish_is_indeterminate_even_with_same_identity() {
    let dir = temp_dir("failed-mutated-publication");
    let path = dir.join("server-settings.json");
    let intended = b"intended settings";
    std::fs::write(&path, b"original settings").unwrap();

    let error = write_atomically_with_before_publish(&path, intended, |_, destination| {
        std::fs::write(destination, b"mutated settings")?;
        Err(std::io::Error::from_raw_os_error(1177))
    })
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::PublicationStateIndeterminate { .. }
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"mutated settings");
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_one_recovery_with_contents(&dir, intended);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn replaced_identity_after_failed_publish_is_indeterminate_even_with_same_bytes() {
    let dir = temp_dir("failed-replaced-identity");
    let path = dir.join("server-settings.json");
    let displaced = dir.join("server-settings.displaced.json");
    let original = b"original settings";
    let intended = b"intended settings";
    std::fs::write(&path, original).unwrap();

    let error = write_atomically_with_before_publish(&path, intended, |_, destination| {
        std::fs::rename(destination, &displaced)?;
        std::fs::write(destination, original)?;
        Err(std::io::Error::from_raw_os_error(1177))
    })
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::PublicationStateIndeterminate { .. }
    ));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read(displaced).unwrap(), original);
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_one_recovery_with_contents(&dir, intended);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn intended_destination_after_failed_publish_is_visible_and_preserves_recovery() {
    let dir = temp_dir("failed-visible-publication");
    let path = dir.join("server-settings.json");
    let intended = b"intended settings";
    std::fs::write(&path, b"original settings").unwrap();

    let error = write_atomically_with_before_publish(&path, intended, |partial, destination| {
        std::fs::remove_file(destination)?;
        std::fs::rename(partial, destination)?;
        Err(std::io::Error::from_raw_os_error(1177))
    })
    .unwrap_err();

    assert!(matches!(
        error,
        ConfigError::PublicationFailedAfterVisibleChange { .. }
    ));
    assert_eq!(std::fs::read(&path).unwrap(), intended);
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_one_recovery_with_contents(&dir, intended);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn normal_success_leaves_no_temp_or_recovery_artifact() {
    let dir = temp_dir("successful-publication-artifacts");
    let path = dir.join("server-settings.json");

    write_atomically_with_before_publish(&path, b"first", |_, _| Ok(())).unwrap();
    write_atomically_with_before_publish(&path, b"second", |_, _| Ok(())).unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"second");
    assert_eq!(partial_files(&dir), Vec::<String>::new());
    assert_eq!(recovery_files(&dir), Vec::<PathBuf>::new());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn recovery_parent_sync_failure_reports_no_confirmed_recovery() {
    let dir = temp_dir("recovery-parent-sync-failure");
    let path = dir.join("server-settings.json");
    let intended = b"intended settings";
    std::fs::write(&path, b"original settings").unwrap();
    let before = snapshot_destination(&path).unwrap();
    let (partial, mut partial_file) = reserve_unique_partial(&path).unwrap();
    partial_file.write_all(intended).unwrap();
    partial_file.sync_all().unwrap();
    drop(partial_file);
    std::fs::remove_file(&path).unwrap();
    let parent_sync_called = AtomicBool::new(false);

    let error = reconcile_publication_failure_with_parent_sync(
        &path,
        &partial,
        intended,
        &before,
        std::io::Error::other("injected publication failure"),
        |recovery| {
            assert_eq!(std::fs::read(recovery)?, intended);
            parent_sync_called.store(true, Ordering::Release);
            Err(std::io::Error::other(
                "injected recovery parent sync failure",
            ))
        },
    );

    assert!(parent_sync_called.load(Ordering::Acquire));
    assert!(matches!(
        error,
        ConfigError::PublicationStateIndeterminate {
            recovery_path: None,
            ..
        }
    ));
    assert!(error.to_string().contains("could not be preserved"));
    assert!(!error.to_string().contains("was preserved at"));
    std::fs::remove_dir_all(dir).ok();
}
