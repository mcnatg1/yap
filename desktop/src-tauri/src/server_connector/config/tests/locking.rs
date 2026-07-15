use super::*;

#[test]
fn load_io_errors_are_reported_as_access_failures() {
    let dir = temp_dir("load-access-error");

    let error = load_from_path(&dir, false).unwrap_err();

    assert!(error
        .to_string()
        .starts_with("Could not access server settings:"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn load_waits_for_the_settings_lock_before_opening_the_destination() {
    let dir = temp_dir("load-lock");
    let path = dir.join("server-settings.json");
    std::fs::write(
        &path,
        r#"{"schemaVersion":1,"enabled":false,"baseUrl":null}"#,
    )
    .unwrap();
    let lock = acquire_settings_lock(&path).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (loaded_tx, loaded_rx) = std::sync::mpsc::channel();
    let reader_path = path.clone();
    let reader = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        loaded_tx.send(load_from_path(&reader_path, false)).unwrap();
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(
        loaded_rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "load opened the destination while the writer lock was held"
    );
    drop(lock);
    assert_eq!(
        loaded_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        ServerSettings::default()
    );
    reader.join().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn missing_load_waits_for_an_in_progress_first_save() {
    let dir = temp_dir("missing-load-lock");
    let path = dir.join("server-settings.json");
    let lock = acquire_settings_lock(&path).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (loaded_tx, loaded_rx) = std::sync::mpsc::channel();
    let reader_path = path.clone();
    let reader = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        loaded_tx.send(load_from_path(&reader_path, false)).unwrap();
    });

    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(
        loaded_rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "missing load bypassed the in-progress writer lock"
    );
    drop(lock);
    assert_eq!(
        loaded_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        ServerSettings::default()
    );
    reader.join().unwrap();
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn indeterminate_error_never_claims_missing_recovery_was_preserved() {
    let error = ConfigError::PublicationStateIndeterminate {
        source: std::io::Error::other("replacement and recovery failed"),
        recovery_path: None,
    };

    assert!(error.settings_may_have_changed());
    assert!(error.to_string().contains("could not be preserved"));
    assert!(!error.to_string().contains("was preserved at"));
}

#[test]
fn locked_save_scavenges_only_matching_abandoned_unique_partials() {
    let dir = temp_dir("stale-unique");
    let path = dir.join("server-settings.json");
    let abandoned = dir.join("server-settings.json.424242.7.part");
    let unrelated = dir.join("server-settings.json.owner.part");
    std::fs::write(&abandoned, "abandoned").unwrap();
    std::fs::write(&unrelated, "keep").unwrap();

    save_to_path(&ServerSettings::default(), &path, false).unwrap();

    assert!(!abandoned.exists());
    assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "keep");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn cross_process_schema2_publisher_helper() {
    let Ok(path) = std::env::var(CROSS_PROCESS_CHILD_PATH) else {
        return;
    };
    let ready = PathBuf::from(std::env::var(CROSS_PROCESS_READY_PATH).unwrap());
    let release = PathBuf::from(std::env::var(CROSS_PROCESS_RELEASE_PATH).unwrap());
    let path = PathBuf::from(path);
    let _lock = acquire_settings_lock(&path).unwrap();
    std::fs::write(&ready, b"locked").unwrap();
    assert!(wait_for_path(&release, Duration::from_secs(10)));
    write_atomically_locked_with_hooks(
        &path,
        CROSS_PROCESS_FUTURE.as_bytes(),
        |_, _| Ok(()),
        |_| Ok(()),
    )
    .unwrap();
}

#[test]
fn v1_writer_rechecks_schema_after_waiting_for_cross_process_lock() {
    let dir = temp_dir("cross-process-schema");
    let path = dir.join("server-settings.json");
    let ready = dir.join("child.ready");
    let release = dir.join("child.release");
    save_to_path(&ServerSettings::default(), &path, false).unwrap();

    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "server_connector::config::tests::locking::cross_process_schema2_publisher_helper",
            "--nocapture",
        ])
        .env(CROSS_PROCESS_CHILD_PATH, &path)
        .env(CROSS_PROCESS_READY_PATH, &ready)
        .env(CROSS_PROCESS_RELEASE_PATH, &release)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if !wait_for_path(&ready, Duration::from_secs(10)) {
        child.kill().ok();
        child.wait().ok();
        panic!("schema2 child did not acquire the settings lock");
    }

    let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        save_to_path_with_hooks(
            &ServerSettings {
                schema_version: CURRENT_SCHEMA_VERSION,
                enabled: true,
                base_url: Some("https://v1-writer.example".into()),
            },
            &writer_path,
            false,
            move || {
                attempted_tx.send(()).unwrap();
                Ok(())
            },
            move || {
                acquired_tx.send(()).unwrap();
                Ok(())
            },
            |_, _| Ok(()),
            |_| Ok(()),
        )
    });
    if attempted_rx.recv_timeout(Duration::from_secs(5)).is_err() {
        child.kill().ok();
        child.wait().ok();
        panic!("v1 writer did not attempt the settings lock");
    }
    assert!(
        acquired_rx
            .recv_timeout(Duration::from_millis(200))
            .is_err(),
        "v1 writer acquired the lock while schema2 still held it"
    );
    std::fs::write(&release, b"publish").unwrap();

    let output = wait_for_child(child, Duration::from_secs(10));
    assert!(
        output.status.success(),
        "child stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    acquired_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        writer.join().unwrap(),
        Err(ConfigError::IncompatibleSchema(2))
    ));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        CROSS_PROCESS_FUTURE
    );

    std::fs::remove_dir_all(dir).ok();
}
