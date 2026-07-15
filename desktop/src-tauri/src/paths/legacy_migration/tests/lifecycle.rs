use super::*;

#[test]
fn copies_only_runtime_entries_and_preserves_legacy_data() {
    let root = test_root(1);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(legacy.join("models")).unwrap();
    fs::create_dir_all(legacy.join("remote-jobs").join("job-legacy")).unwrap();
    fs::write(legacy.join("models").join("model.bin"), b"model").unwrap();
    fs::write(
        legacy
            .join("remote-jobs")
            .join("job-legacy")
            .join("private.pcm"),
        b"private recording",
    )
    .unwrap();
    fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();
    fs::write(
        legacy.join("recording-native-selection-registry.json"),
        b"native selection",
    )
    .unwrap();
    fs::write(
        legacy.join("server-origin-approval.json"),
        b"server approval",
    )
    .unwrap();
    fs::write(legacy.join("yap-desktop.exe"), b"installed binary").unwrap();

    let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

    assert_eq!(outcome, LegacyMigrationOutcome::Migrated { entries: 5 });
    assert_eq!(
        fs::read(canonical.join("models").join("model.bin")).unwrap(),
        b"model"
    );
    assert_eq!(
        fs::read(
            canonical
                .join("remote-jobs")
                .join("job-legacy")
                .join("private.pcm")
        )
        .unwrap(),
        b"private recording"
    );
    assert_eq!(
        fs::read(canonical.join("recording-native-selection-registry.json")).unwrap(),
        b"native selection"
    );
    assert_eq!(
        fs::read(canonical.join("server-origin-approval.json")).unwrap(),
        b"server approval"
    );
    assert!(legacy.join("yap-desktop.exe").is_file());
    assert_eq!(fs::read(legacy.join("jobs.sqlite3")).unwrap(), b"ledger");
    assert!(canonical.join(MIGRATION_COMPLETION_FILE).is_file());

    fs::write(canonical.join("jobs.sqlite3"), b"canonical update").unwrap();
    assert_eq!(
        migrate_legacy_entries(&legacy, &canonical).unwrap(),
        LegacyMigrationOutcome::NotNeeded
    );
    assert_eq!(
        fs::read(canonical.join("jobs.sqlite3")).unwrap(),
        b"canonical update"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_logs_do_not_block_authoritative_data_migration() {
    let root = test_root(12);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(legacy.join("logs")).unwrap();
    fs::create_dir_all(canonical.join("logs")).unwrap();
    fs::write(
        legacy.join("logs").join("legacy.log"),
        b"legacy diagnostics",
    )
    .unwrap();
    fs::write(
        canonical.join("logs").join("yap.log"),
        b"canonical diagnostics",
    )
    .unwrap();
    fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();

    let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

    assert_eq!(outcome, LegacyMigrationOutcome::Migrated { entries: 1 });
    assert_eq!(fs::read(canonical.join("jobs.sqlite3")).unwrap(), b"ledger");
    assert_eq!(
        fs::read(canonical.join("logs").join("yap.log")).unwrap(),
        b"canonical diagnostics"
    );
    assert_eq!(
        fs::read(legacy.join("logs").join("legacy.log")).unwrap(),
        b"legacy diagnostics"
    );
    assert_eq!(fs::read(legacy.join("jobs.sqlite3")).unwrap(), b"ledger");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_conflicts_before_publishing_anything() {
    let root = test_root(2);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(legacy.join("jobs.sqlite3"), b"legacy ledger").unwrap();
    fs::write(legacy.join("live-settings.json"), b"legacy settings").unwrap();
    fs::write(canonical.join("live-settings.json"), b"new settings").unwrap();

    let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(legacy.join("jobs.sqlite3").is_file());
    assert!(!canonical.join("jobs.sqlite3").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn does_not_create_canonical_storage_without_runtime_data() {
    let root = test_root(3);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("uninstall.exe"), b"uninstaller").unwrap();

    let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

    assert_eq!(outcome, LegacyMigrationOutcome::NotNeeded);
    assert!(!canonical.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_migrations_serialize_and_converge() {
    let root = test_root(5);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(legacy.join("models")).unwrap();
    fs::write(legacy.join("models").join("model.bin"), b"model").unwrap();
    fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();
    let ready = Arc::new(Barrier::new(2));

    let handles = (0..2)
        .map(|_| {
            let legacy = legacy.clone();
            let canonical = canonical.clone();
            let ready = Arc::clone(&ready);
            std::thread::spawn(move || {
                migrate_legacy_entries_with_ready_hook(&legacy, &canonical, || ready.wait())
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, LegacyMigrationOutcome::Migrated { .. }))
            .count(),
        1
    );
    assert_eq!(
        fs::read(canonical.join("models").join("model.bin")).unwrap(),
        b"model"
    );
    assert_eq!(fs::read(canonical.join("jobs.sqlite3")).unwrap(), b"ledger");
    assert!(legacy.join("models").is_dir());
    assert!(legacy.join("jobs.sqlite3").is_file());
    assert!(canonical.join(MIGRATION_COMPLETION_FILE).is_file());
    fs::remove_dir_all(root).unwrap();
}
