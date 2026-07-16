use super::*;

#[test]
fn interrupted_staging_is_verified_and_recovered_before_retry() {
    let root = test_root(8);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let stale_stage = canonical.join(".legacy-migration-stage-interrupted");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&stale_stage).unwrap();
    fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();
    fs::write(stale_stage.join("jobs.sqlite3"), b"ledger").unwrap();

    let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

    assert_eq!(outcome, LegacyMigrationOutcome::Migrated { entries: 1 });
    assert_eq!(fs::read(canonical.join("jobs.sqlite3")).unwrap(), b"ledger");
    assert!(!stale_stage.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_retirement_is_verified_and_removed_before_early_return() {
    let root = test_root(9);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let stale_retirement = legacy.join(".yap-runtime-migrated-interrupted");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&stale_retirement).unwrap();
    fs::create_dir_all(&canonical).unwrap();
    fs::write(stale_retirement.join("jobs.sqlite3"), b"ledger").unwrap();
    fs::write(canonical.join("jobs.sqlite3"), b"ledger").unwrap();

    let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

    assert_eq!(outcome, LegacyMigrationOutcome::NotNeeded);
    assert!(!stale_retirement.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_failure_is_propagated_with_the_original_error() {
    let original = io::Error::new(io::ErrorKind::InvalidData, "copy verification failed");
    let combined = cleanup_after_error_with(Path::new("stale-stage"), original, |_| {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "in use"))
    });

    assert_eq!(combined.kind(), io::ErrorKind::InvalidData);
    assert!(combined.to_string().contains("copy verification failed"));
    assert!(combined.to_string().contains("cleanup also failed"));
}

#[test]
fn unverifiable_staging_residue_is_preserved() {
    let root = test_root(11);
    let legacy = root.join("legacy");
    let canonical = root.join("canonical");
    let stale_stage = canonical.join(".legacy-migration-stage-only-copy");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&stale_stage).unwrap();
    fs::write(stale_stage.join("jobs.sqlite3"), b"only copy").unwrap();

    let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        fs::read(stale_stage.join("jobs.sqlite3")).unwrap(),
        b"only copy"
    );
    fs::remove_dir_all(root).unwrap();
}
