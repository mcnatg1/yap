mod platform;
mod recovery;
mod secure_tree;

use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::LegacyMigrationOutcome;
use platform::{open_migration_lock, open_regular_file_read, rename_no_replace, DirectoryLease};
use recovery::{cleanup_after_error, migration_residue_directories, recover_migration_residue};
use secure_tree::{
    copy_tree_verified, create_unique_directory, create_unique_file, ensure_normal_directory,
    metadata_if_present, metadata_is_link_or_reparse, trees_equal, validate_regular_tree,
};

const MIGRATION_STAGE_PREFIX: &str = ".legacy-migration-stage";
const MIGRATION_RETIREMENT_PREFIX: &str = ".yap-runtime-migrated";
const MIGRATION_COMPLETION_FILE: &str = ".legacy-migration-v1.complete";
const MIGRATION_COMPLETION_CONTENT: &[u8] = b"yap-legacy-migration-v1\n";
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn migrate_legacy_entries(
    legacy: &Path,
    canonical: &Path,
) -> io::Result<LegacyMigrationOutcome> {
    migrate_legacy_entries_with_ready_hook(legacy, canonical, || {})
}

fn migrate_legacy_entries_with_ready_hook<F, R>(
    legacy: &Path,
    canonical: &Path,
    ready: F,
) -> io::Result<LegacyMigrationOutcome>
where
    F: FnOnce() -> R,
{
    let initial_entries = discover_legacy_entries(legacy, canonical)?;
    let initial_staging = migration_residue_directories(canonical, MIGRATION_STAGE_PREFIX)?;
    let initial_retirement = migration_residue_directories(legacy, MIGRATION_RETIREMENT_PREFIX)?;
    if initial_entries.is_empty() && initial_staging.is_empty() && initial_retirement.is_empty() {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }

    ensure_normal_directory(canonical)?;
    let lock_file = open_migration_lock(&canonical.join(".legacy-migration.lock"))?;
    ready();
    lock_with_timeout(&lock_file, MIGRATION_LOCK_TIMEOUT)?;
    migrate_legacy_entries_locked(legacy, canonical)
}

fn migrate_legacy_entries_locked(
    legacy: &Path,
    canonical: &Path,
) -> io::Result<LegacyMigrationOutcome> {
    recover_migration_residue(legacy, canonical)?;
    if migration_is_complete(canonical)? {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }
    let entries = discover_legacy_entries(legacy, canonical)?;
    if entries.is_empty() {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }

    let mut unpublished = Vec::new();
    for (source, destination) in &entries {
        validate_regular_tree(source)?;
        match metadata_if_present(destination)? {
            Some(_) => {
                validate_regular_tree(destination)?;
                if !trees_equal(source, destination)? {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "legacy Yap runtime entry conflicts with canonical app data: {}",
                            destination.display()
                        ),
                    ));
                }
            }
            None => unpublished.push((source.clone(), destination.clone())),
        }
    }

    let staging = if unpublished.is_empty() {
        None
    } else {
        Some(create_unique_directory(canonical, MIGRATION_STAGE_PREFIX)?)
    };
    if let Some(staging) = &staging {
        let stage_result = (|| {
            for (source, _) in &unpublished {
                let name = source.file_name().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("legacy entry has no file name: {}", source.display()),
                    )
                })?;
                copy_tree_verified(source, &staging.join(name))?;
            }
            Ok::<(), io::Error>(())
        })();
        if let Err(error) = stage_result {
            return Err(cleanup_after_error(staging, error));
        }

        let publish_result = (|| {
            for (source, destination) in &unpublished {
                if metadata_if_present(destination)?.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "canonical app-data destination appeared during migration: {}",
                            destination.display()
                        ),
                    ));
                }
                let staged = staging.join(source.file_name().expect("validated legacy entry name"));
                rename_no_replace(&staged, destination)?;
                if !trees_equal(source, destination)? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "published legacy entry failed verification: {}",
                            destination.display()
                        ),
                    ));
                }
            }
            fs::remove_dir(staging)?;
            Ok::<(), io::Error>(())
        })();
        if let Err(error) = publish_result {
            return Err(cleanup_after_error(staging, error));
        }
    }

    for (source, destination) in &entries {
        if !trees_equal(source, destination)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "canonical app-data entry changed before migration completion: {}",
                    destination.display()
                ),
            ));
        }
    }

    publish_migration_completion(canonical)?;

    Ok(LegacyMigrationOutcome::Migrated {
        entries: entries.len(),
    })
}

fn migration_is_complete(canonical: &Path) -> io::Result<bool> {
    let marker = canonical.join(MIGRATION_COMPLETION_FILE);
    let Some(metadata) = metadata_if_present(&marker)? else {
        return Ok(false);
    };
    if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy migration completion marker is not a normal file: {}",
                marker.display()
            ),
        ));
    }
    let mut file = open_regular_file_read(&marker)?;
    let mut content = Vec::with_capacity(MIGRATION_COMPLETION_CONTENT.len() + 1);
    Read::by_ref(&mut file)
        .take((MIGRATION_COMPLETION_CONTENT.len() + 1) as u64)
        .read_to_end(&mut content)?;
    if content != MIGRATION_COMPLETION_CONTENT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy migration completion marker is invalid: {}",
                marker.display()
            ),
        ));
    }
    Ok(true)
}

fn publish_migration_completion(canonical: &Path) -> io::Result<()> {
    let marker = canonical.join(MIGRATION_COMPLETION_FILE);
    if migration_is_complete(canonical)? {
        return Ok(());
    }

    let (temporary, mut file) = create_unique_file(canonical, ".legacy-migration-complete")?;
    let publication = (|| {
        file.write_all(MIGRATION_COMPLETION_CONTENT)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        rename_no_replace(&temporary, &marker)?;
        Ok::<(), io::Error>(())
    })();
    match publication {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary);
            if migration_is_complete(canonical)? {
                Ok(())
            } else {
                Err(error)
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn lock_with_timeout(file: &File, timeout: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::WouldBlock) => {
                let elapsed = started.elapsed();
                if elapsed >= timeout {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "another Yap process is still migrating legacy app data",
                    ));
                }
                std::thread::sleep(
                    timeout
                        .saturating_sub(elapsed)
                        .min(Duration::from_millis(50)),
                );
            }
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
        }
    }
}

fn discover_legacy_entries(legacy: &Path, canonical: &Path) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    if metadata_if_present(legacy)?.is_none() {
        return Ok(Vec::new());
    }

    let directory = DirectoryLease::open(legacy)?;
    let mut sources = Vec::new();
    for name in directory.sorted_entry_names()? {
        if is_migratable_legacy_runtime_entry(&name) {
            sources.push(legacy.join(name));
        }
    }
    sources.sort();
    Ok(sources
        .into_iter()
        .map(|source| {
            let destination = canonical.join(
                source
                    .file_name()
                    .expect("directory entry always has a file name"),
            );
            (source, destination)
        })
        .collect())
}

fn is_migratable_legacy_runtime_entry(name: &OsStr) -> bool {
    // Canonical logging can already have created a different logs tree. Logs
    // are diagnostic residue, not authoritative runtime state, so preserve the
    // legacy copy in place instead of letting it block the data migration.
    name.to_str() != Some("logs") && is_recognized_legacy_runtime_entry(name)
}

fn is_recognized_legacy_runtime_entry(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    matches!(
        name,
        "models"
            | "live-recordings"
            | "remote-jobs"
            | "logs"
            | "install-id"
            | "local-fallback.disabled"
            | "compute-target.txt"
    ) || name == "jobs.sqlite3"
        || name == "jobs.sqlite3-shm"
        || name == "jobs.sqlite3-wal"
        || name.starts_with("live-settings.json")
        || name.starts_with("server-settings.json")
        || name.starts_with("server-origin-approval.json")
        || name.starts_with("recording-playback-registry.json")
        || name.starts_with("recording-job-playback-registry.json")
        || name.starts_with("recording-native-selection-registry.json")
}

#[cfg(test)]
mod tests;
