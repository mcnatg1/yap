use super::*;

#[test]
fn verified_staging_copy_preserves_source_and_matches_every_byte() {
    let root = test_root(4);
    let source = root.join("source");
    let staged = root.join("staged");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("nested").join("model.bin"), b"model bytes").unwrap();
    fs::write(source.join("settings.json"), b"{\"enabled\":true}").unwrap();

    copy_tree_verified(&source, &staged).unwrap();

    assert!(source.join("nested").join("model.bin").is_file());
    assert!(trees_equal(&source, &staged).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn tree_work_is_rejected_at_configured_depth_entry_and_byte_limits() {
    let root = test_root(16);
    let tree = root.join("tree");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(tree.join("nested").join("deep")).unwrap();
    fs::write(tree.join("first"), b"12345").unwrap();
    fs::write(tree.join("second"), b"67890").unwrap();

    let depth_error =
        validate_regular_tree_with_limits(&tree, TreeLimits::bounded(1, 100, 1_000)).unwrap_err();
    let entry_error =
        validate_regular_tree_with_limits(&tree, TreeLimits::bounded(100, 2, 1_000)).unwrap_err();
    let byte_error =
        validate_regular_tree_with_limits(&tree, TreeLimits::bounded(100, 100, 4)).unwrap_err();

    assert_eq!(depth_error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(entry_error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(byte_error.kind(), io::ErrorKind::InvalidData);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn atomic_publication_never_replaces_a_late_destination() {
    let root = test_root(13);
    let source = root.join("staged");
    let destination = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(&source, b"staged bytes").unwrap();
    fs::write(&destination, b"late canonical bytes").unwrap();

    let error = rename_no_replace(&source, &destination).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&destination).unwrap(), b"late canonical bytes");
    assert_eq!(fs::read(&source).unwrap(), b"staged bytes");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn migration_lock_links_are_rejected_without_following_them() {
    let root = test_root(14);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let outside = root.join("outside-lock");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();
    fs::write(&outside, b"outside").unwrap();
    if let Err(error) = create_file_symlink(&outside, &canonical.join(".legacy-migration.lock")) {
        if test_symlink_is_unavailable(&error) {
            fs::remove_dir_all(root).unwrap();
            return;
        }
        panic!("could not create test symlink: {error}");
    }

    let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(outside).unwrap(), b"outside");
    assert!(!canonical.join("jobs.sqlite3").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn open_migration_lock_cannot_be_renamed_away() {
    let root = test_root(15);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let lock_path = root.join("lock");
    let moved_path = root.join("moved-lock");
    let lock = open_migration_lock(&lock_path).unwrap();

    assert!(fs::rename(&lock_path, &moved_path).is_err());
    assert!(lock_path.is_file());

    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn source_leases_block_in_place_writes_and_directory_retargeting() {
    let root = test_root(17);
    let source_dir = root.join("source");
    let moved_dir = root.join("moved");
    let source_file = source_dir.join("model.bin");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(&source_file, b"original").unwrap();

    let directory = DirectoryLease::open(&source_dir).unwrap();
    let file = open_regular_file_read(&source_file).unwrap();

    assert!(fs::OpenOptions::new()
        .write(true)
        .open(&source_file)
        .is_err());
    assert!(fs::rename(&source_dir, &moved_dir).is_err());
    assert_eq!(fs::read(&source_file).unwrap(), b"original");

    drop(file);
    drop(directory);
    fs::write(&source_file, b"updated").unwrap();
    assert_eq!(fs::read(&source_file).unwrap(), b"updated");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_nested_links_before_publication() {
    let root = test_root(6);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(legacy.join("models")).unwrap();
    fs::write(root.join("outside-model.bin"), b"outside").unwrap();
    if let Err(error) = create_file_symlink(
        &root.join("outside-model.bin"),
        &legacy.join("models").join("linked-model.bin"),
    ) {
        if test_symlink_is_unavailable(&error) {
            fs::remove_dir_all(root).unwrap();
            return;
        }
        panic!("could not create test symlink: {error}");
    }

    let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!canonical.join("models").exists());
    assert!(legacy.join("models").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dangling_destination_links_are_conflicts() {
    let root = test_root(7);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(legacy.join("jobs.sqlite3"), b"legacy ledger").unwrap();
    if let Err(error) = create_file_symlink(
        &root.join("missing-target"),
        &canonical.join("jobs.sqlite3"),
    ) {
        if test_symlink_is_unavailable(&error) {
            fs::remove_dir_all(root).unwrap();
            return;
        }
        panic!("could not create test symlink: {error}");
    }

    let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(fs::symlink_metadata(canonical.join("jobs.sqlite3")).is_ok());
    assert!(legacy.join("jobs.sqlite3").is_file());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn migration_lock_wait_is_bounded() {
    let root = test_root(10);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let lock_path = root.join("lock");
    let first = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    let second = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .unwrap();
    first.lock().unwrap();

    let error = lock_with_timeout(&second, Duration::from_millis(20)).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    drop(first);
    fs::remove_dir_all(root).unwrap();
}
