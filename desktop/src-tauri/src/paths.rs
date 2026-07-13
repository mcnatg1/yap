use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

const PRODUCTION_IDENTIFIER: &str = "com.mcnatg1.yap";
const LEGACY_APP_NAME: &str = "Yap";
const MIGRATION_STAGE_PREFIX: &str = ".legacy-migration-stage";
const MIGRATION_RETIREMENT_PREFIX: &str = ".yap-runtime-migrated";
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyMigrationOutcome {
    NotNeeded,
    Migrated { entries: usize },
}

pub(crate) fn app_data_dir() -> PathBuf {
    app_data_dir_from_root(|key| std::env::var(key).ok(), dirs::data_dir())
}

pub(crate) fn app_data_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    let data_root = data_root_from_env(&env).or_else(dirs::data_dir);
    app_data_dir_from_root(env, data_root)
}

fn app_data_dir_from_root<F>(env: F, data_root: Option<PathBuf>) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(override_dir) = absolute_env_path(&env, "YAP_APP_DATA_DIR") {
        return override_dir;
    }
    data_root
        .expect("Tauri app-data root is unavailable")
        .join(PRODUCTION_IDENTIFIER)
}

pub(crate) fn migrate_legacy_app_data() -> io::Result<LegacyMigrationOutcome> {
    let env = |key: &str| std::env::var(key).ok();
    if absolute_env_path(&env, "YAP_APP_DATA_DIR").is_some() {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }
    let canonical_root = dirs::data_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Tauri app-data root is unavailable",
        )
    })?;
    let Some(legacy) = legacy_app_data_dir_from(&env) else {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    };
    migrate_legacy_entries(&legacy, &canonical_root.join(PRODUCTION_IDENTIFIER))
}

fn legacy_app_data_dir_from<F>(env: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    absolute_env_path(env, "LOCALAPPDATA")
        .map(|root| root.join(LEGACY_APP_NAME))
        .or_else(|| absolute_env_path(env, "XDG_DATA_HOME").map(|root| root.join(LEGACY_APP_NAME)))
        .or_else(|| {
            absolute_env_path(env, "HOME")
                .map(|home| home.join(".local").join("share").join(LEGACY_APP_NAME))
        })
}

fn migrate_legacy_entries(legacy: &Path, canonical: &Path) -> io::Result<LegacyMigrationOutcome> {
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
    let lock_path = canonical.join(".legacy-migration.lock");
    if let Some(metadata) = metadata_if_present(&lock_path)? {
        if !metadata.is_file() || metadata_is_link_or_reparse(&metadata) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "legacy migration lock is not a normal file: {}",
                    lock_path.display()
                ),
            ));
        }
    }
    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    let lock_metadata = fs::symlink_metadata(&lock_path)?;
    if !lock_metadata.is_file() || metadata_is_link_or_reparse(&lock_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy migration lock is not a normal file: {}",
                lock_path.display()
            ),
        ));
    }
    ready();
    lock_with_timeout(&lock_file, MIGRATION_LOCK_TIMEOUT)?;
    migrate_legacy_entries_locked(legacy, canonical)
}

fn migrate_legacy_entries_locked(
    legacy: &Path,
    canonical: &Path,
) -> io::Result<LegacyMigrationOutcome> {
    recover_migration_residue(legacy, canonical)?;
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
                fs::rename(&staged, destination)?;
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
                    "canonical app-data entry changed before source retirement: {}",
                    destination.display()
                ),
            ));
        }
    }

    let retirement = create_unique_directory(legacy, MIGRATION_RETIREMENT_PREFIX)?;
    for (source, _) in &entries {
        let retired = retirement.join(source.file_name().expect("validated legacy entry name"));
        if let Err(error) = fs::rename(source, &retired) {
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "canonical data is verified, but legacy source retirement failed for {}: {error}",
                    source.display()
                ),
            ));
        }
    }
    fs::remove_dir_all(&retirement)?;

    Ok(LegacyMigrationOutcome::Migrated {
        entries: entries.len(),
    })
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

fn recover_migration_residue(legacy: &Path, canonical: &Path) -> io::Result<()> {
    recover_staging_residue(legacy, canonical)?;
    recover_retirement_residue(legacy, canonical)
}

fn recover_staging_residue(legacy: &Path, canonical: &Path) -> io::Result<()> {
    for residue in migration_residue_directories(canonical, MIGRATION_STAGE_PREFIX)? {
        validate_regular_tree(&residue)?;
        for staged in sorted_entries(&residue)? {
            let name = staged.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("staging residue has no entry name: {}", staged.display()),
                )
            })?;
            if !is_legacy_runtime_entry(name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "staging residue contains an unrecognized entry: {}",
                        staged.display()
                    ),
                ));
            }
            validate_regular_tree(&staged)?;
            let source = legacy.join(name);
            let destination = canonical.join(name);
            let mut verified_copy = false;
            for candidate in [&source, &destination] {
                if metadata_if_present(candidate)?.is_some() {
                    validate_regular_tree(candidate)?;
                    if !trees_equal(&staged, candidate)? {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "staging residue does not match {}: {}",
                                candidate.display(),
                                staged.display()
                            ),
                        ));
                    }
                    verified_copy = true;
                }
            }
            if !verified_copy {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "staging residue is the only remaining copy and was preserved: {}",
                        staged.display()
                    ),
                ));
            }
            remove_regular_tree(&staged)?;
        }
        fs::remove_dir(&residue)?;
    }
    Ok(())
}

fn recover_retirement_residue(legacy: &Path, canonical: &Path) -> io::Result<()> {
    for residue in migration_residue_directories(legacy, MIGRATION_RETIREMENT_PREFIX)? {
        validate_regular_tree(&residue)?;
        for retired in sorted_entries(&residue)? {
            let name = retired.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "retirement residue has no entry name: {}",
                        retired.display()
                    ),
                )
            })?;
            if !is_legacy_runtime_entry(name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "retirement residue contains an unrecognized entry: {}",
                        retired.display()
                    ),
                ));
            }
            validate_regular_tree(&retired)?;
            let source = legacy.join(name);
            let destination = canonical.join(name);
            let source_exists = metadata_if_present(&source)?.is_some();
            let destination_exists = metadata_if_present(&destination)?.is_some();

            if source_exists {
                validate_regular_tree(&source)?;
                if !trees_equal(&retired, &source)? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "retirement residue conflicts with restored legacy data: {}",
                            retired.display()
                        ),
                    ));
                }
            }
            if destination_exists {
                validate_regular_tree(&destination)?;
                if !trees_equal(&retired, &destination)? {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "retirement residue conflicts with canonical app data: {}",
                            retired.display()
                        ),
                    ));
                }
            }

            match (source_exists, destination_exists) {
                (false, false) => fs::rename(&retired, &source)?,
                (true, _) | (false, true) => remove_regular_tree(&retired)?,
            }
        }
        fs::remove_dir(&residue)?;
    }
    Ok(())
}

fn migration_residue_directories(parent: &Path, prefix: &str) -> io::Result<Vec<PathBuf>> {
    let Some(metadata) = metadata_if_present(parent)? else {
        return Ok(Vec::new());
    };
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "migration parent is not a normal directory: {}",
                parent.display()
            ),
        ));
    }
    let mut residues = fs::read_dir(parent)?
        .filter_map(|entry| match entry {
            Ok(entry)
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with(prefix)) =>
            {
                Some(Ok(entry.path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, io::Error>>()?;
    residues.sort();
    Ok(residues)
}

fn sorted_entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    Ok(entries)
}

fn remove_regular_tree(path: &Path) -> io::Result<()> {
    validate_regular_tree(path)?;
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn cleanup_after_error(path: &Path, error: io::Error) -> io::Error {
    cleanup_after_error_with(path, error, |path| fs::remove_dir_all(path))
}

fn cleanup_after_error_with<F>(path: &Path, error: io::Error, cleanup: F) -> io::Error
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    match cleanup(path) {
        Ok(()) => error,
        Err(cleanup_error) => io::Error::new(
            error.kind(),
            format!(
                "{error}; migration residue cleanup also failed for {}: {cleanup_error}",
                path.display()
            ),
        ),
    }
}

fn discover_legacy_entries(legacy: &Path, canonical: &Path) -> io::Result<Vec<(PathBuf, PathBuf)>> {
    let Some(metadata) = metadata_if_present(legacy)? else {
        return Ok(Vec::new());
    };
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy Yap data root is not a normal directory: {}",
                legacy.display()
            ),
        ));
    }

    let mut sources = Vec::new();
    for entry in fs::read_dir(legacy)? {
        let entry = entry?;
        if is_legacy_runtime_entry(&entry.file_name()) {
            sources.push(entry.path());
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

fn ensure_normal_directory(path: &Path) -> io::Result<()> {
    if metadata_if_present(path)?.is_none() {
        fs::create_dir_all(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata_is_link_or_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("path is not a normal directory: {}", path.display()),
        ));
    }
    Ok(())
}

fn metadata_if_present(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn validate_regular_tree(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "migration tree contains a link or reparse point: {}",
                path.display()
            ),
        ));
    }
    if metadata.is_file() {
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("migration tree contains a special file: {}", path.display()),
        ));
    }
    for entry in fs::read_dir(path)? {
        validate_regular_tree(&entry?.path())?;
    }
    Ok(())
}

fn copy_tree_verified(source: &Path, destination: &Path) -> io::Result<()> {
    validate_regular_tree(source)?;
    if metadata_if_present(destination)?.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "staging destination already exists: {}",
                destination.display()
            ),
        ));
    }
    copy_tree(source, destination)?;
    if !trees_equal(source, destination)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "staged copy does not match its source: {}",
                source.display()
            ),
        ));
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.is_file() {
        let mut input = File::open(source)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)?;
        io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        return Ok(());
    }
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn trees_equal(left: &Path, right: &Path) -> io::Result<bool> {
    let left_metadata = fs::symlink_metadata(left)?;
    let right_metadata = fs::symlink_metadata(right)?;
    if metadata_is_link_or_reparse(&left_metadata) || metadata_is_link_or_reparse(&right_metadata) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cannot compare a migration tree containing links or reparse points",
        ));
    }
    if left_metadata.is_file() != right_metadata.is_file()
        || left_metadata.is_dir() != right_metadata.is_dir()
    {
        return Ok(false);
    }
    if left_metadata.is_file() {
        return Ok(sha256_file(left)? == sha256_file(right)?);
    }
    if !left_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "cannot compare special files in a migration tree",
        ));
    }
    let left_names = sorted_entry_names(left)?;
    let right_names = sorted_entry_names(right)?;
    if left_names != right_names {
        return Ok(false);
    }
    for name in left_names {
        if !trees_equal(&left.join(&name), &right.join(&name))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sorted_entry_names(path: &Path) -> io::Result<Vec<OsString>> {
    let mut names = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn create_unique_directory(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..1000_u16 {
        let candidate = parent.join(format!("{prefix}-{}-{nonce}-{attempt}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!(
            "could not allocate migration directory under {}",
            parent.display()
        ),
    ))
}

fn is_legacy_runtime_entry(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    matches!(
        name,
        "models"
            | "live-recordings"
            | "logs"
            | "install-id"
            | "local-fallback.disabled"
            | "compute-target.txt"
    ) || name == "jobs.sqlite3"
        || name == "jobs.sqlite3-shm"
        || name == "jobs.sqlite3-wal"
        || name.starts_with("live-settings.json")
        || name.starts_with("server-settings.json")
        || name.starts_with("recording-playback-registry.json")
        || name.starts_with("recording-job-playback-registry.json")
}

fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

#[cfg(windows)]
fn data_root_from_env<F>(env: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    absolute_env_path(env, "APPDATA")
}

#[cfg(target_os = "macos")]
fn data_root_from_env<F>(env: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    absolute_env_path(env, "HOME").map(|home| home.join("Library").join("Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn data_root_from_env<F>(env: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    absolute_env_path(env, "XDG_DATA_HOME")
        .or_else(|| absolute_env_path(env, "HOME").map(|home| home.join(".local").join("share")))
}

pub(crate) fn absolute_env_path<F>(env: &F, key: &str) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    let path = PathBuf::from(env(key)?);
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[cfg(unix)]
    fn create_file_symlink(source: &Path, destination: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(source, destination)
    }

    #[cfg(windows)]
    fn create_file_symlink(source: &Path, destination: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_file(source, destination)
    }

    fn test_symlink_is_unavailable(error: &io::Error) -> bool {
        cfg!(windows)
            && (error.kind() == io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314))
    }

    #[test]
    fn app_data_dir_prefers_explicit_override() {
        let override_dir = std::env::temp_dir().join("yap-test-data");
        let local = std::env::temp_dir().join("local-data");
        let dir = app_data_dir_from(|key| match key {
            "YAP_APP_DATA_DIR" => Some(override_dir.display().to_string()),
            "APPDATA" => Some(local.display().to_string()),
            _ => None,
        });

        assert_eq!(dir, override_dir);
    }

    #[test]
    fn app_data_dir_uses_tauri_data_root_and_identifier() {
        let data_root = std::env::temp_dir().join("tauri-data");
        let dir = app_data_dir_from_root(|_| None, Some(data_root.clone()));

        assert_eq!(dir, data_root.join("com.mcnatg1.yap"));
    }

    #[test]
    fn app_data_dir_keeps_production_data_namespace() {
        let local = std::env::temp_dir().join("local-data");
        let dir = app_data_dir_from_root(|_| None, Some(local.clone()));

        assert_eq!(dir, local.join("com.mcnatg1.yap"));
    }

    #[test]
    #[should_panic(expected = "Tauri app-data root is unavailable")]
    fn missing_tauri_data_root_fails_closed() {
        let _ = app_data_dir_from_root(|_| None, None);
    }

    #[test]
    fn absolute_env_path_rejects_relative_values() {
        let env = |key: &str| (key == "YAP_MODELS_DIR").then(|| "models".to_string());

        assert_eq!(absolute_env_path(&env, "YAP_MODELS_DIR"), None);
    }

    #[test]
    fn legacy_path_matches_the_previous_local_app_data_namespace() {
        let local = std::env::temp_dir().join("legacy-local-data");
        let xdg = std::env::temp_dir().join("legacy-xdg-data");
        let path = legacy_app_data_dir_from(&|key| match key {
            "LOCALAPPDATA" => Some(local.display().to_string()),
            "XDG_DATA_HOME" => Some(xdg.display().to_string()),
            _ => None,
        });

        assert_eq!(path, Some(local.join("Yap")));
    }

    #[test]
    fn legacy_migration_moves_only_runtime_entries() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            1
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(legacy.join("models")).unwrap();
        std::fs::write(legacy.join("models").join("model.bin"), b"model").unwrap();
        std::fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();
        std::fs::write(legacy.join("yap-desktop.exe"), b"installed binary").unwrap();

        let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

        assert_eq!(outcome, LegacyMigrationOutcome::Migrated { entries: 2 });
        assert_eq!(
            std::fs::read(canonical.join("models").join("model.bin")).unwrap(),
            b"model"
        );
        assert_eq!(
            std::fs::read(canonical.join("jobs.sqlite3")).unwrap(),
            b"ledger"
        );
        assert!(legacy.join("yap-desktop.exe").is_file());
        assert!(!legacy.join("jobs.sqlite3").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_migration_rejects_conflicts_before_moving_anything() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            2
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::write(legacy.join("jobs.sqlite3"), b"legacy ledger").unwrap();
        std::fs::write(legacy.join("live-settings.json"), b"legacy settings").unwrap();
        std::fs::write(canonical.join("live-settings.json"), b"new settings").unwrap();

        let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(legacy.join("jobs.sqlite3").is_file());
        assert!(!canonical.join("jobs.sqlite3").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_migration_does_not_create_canonical_storage_without_runtime_data() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            3
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("uninstall.exe"), b"uninstaller").unwrap();

        let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

        assert_eq!(outcome, LegacyMigrationOutcome::NotNeeded);
        assert!(!canonical.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_staging_copy_preserves_source_and_matches_every_byte() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            4
        ));
        let source = root.join("source");
        let staged = root.join("staged");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(source.join("nested").join("model.bin"), b"model bytes").unwrap();
        std::fs::write(source.join("settings.json"), b"{\"enabled\":true}").unwrap();

        copy_tree_verified(&source, &staged).unwrap();

        assert!(source.join("nested").join("model.bin").is_file());
        assert!(trees_equal(&source, &staged).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_migrations_serialize_and_converge() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            5
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(legacy.join("models")).unwrap();
        std::fs::write(legacy.join("models").join("model.bin"), b"model").unwrap();
        std::fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();
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
            std::fs::read(canonical.join("models").join("model.bin")).unwrap(),
            b"model"
        );
        assert_eq!(
            std::fs::read(canonical.join("jobs.sqlite3")).unwrap(),
            b"ledger"
        );
        assert!(!legacy.join("models").exists());
        assert!(!legacy.join("jobs.sqlite3").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_migration_rejects_nested_links_before_publication() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            6
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(legacy.join("models")).unwrap();
        std::fs::write(root.join("outside-model.bin"), b"outside").unwrap();
        if let Err(error) = create_file_symlink(
            &root.join("outside-model.bin"),
            &legacy.join("models").join("linked-model.bin"),
        ) {
            if test_symlink_is_unavailable(&error) {
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("could not create test symlink: {error}");
        }

        let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(!canonical.join("models").exists());
        assert!(legacy.join("models").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_migration_treats_a_dangling_destination_link_as_a_conflict() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            7
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::write(legacy.join("jobs.sqlite3"), b"legacy ledger").unwrap();
        if let Err(error) = create_file_symlink(
            &root.join("missing-target"),
            &canonical.join("jobs.sqlite3"),
        ) {
            if test_symlink_is_unavailable(&error) {
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("could not create test symlink: {error}");
        }

        let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(fs::symlink_metadata(canonical.join("jobs.sqlite3")).is_ok());
        assert!(legacy.join("jobs.sqlite3").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_staging_is_verified_and_recovered_before_retry() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            8
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let stale_stage = canonical.join(".legacy-migration-stage-interrupted");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&stale_stage).unwrap();
        std::fs::write(legacy.join("jobs.sqlite3"), b"ledger").unwrap();
        std::fs::write(stale_stage.join("jobs.sqlite3"), b"ledger").unwrap();

        let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

        assert_eq!(outcome, LegacyMigrationOutcome::Migrated { entries: 1 });
        assert_eq!(
            std::fs::read(canonical.join("jobs.sqlite3")).unwrap(),
            b"ledger"
        );
        assert!(!stale_stage.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_retirement_is_verified_and_removed_before_early_return() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            9
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let stale_retirement = legacy.join(".yap-runtime-migrated-interrupted");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&stale_retirement).unwrap();
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::write(stale_retirement.join("jobs.sqlite3"), b"ledger").unwrap();
        std::fs::write(canonical.join("jobs.sqlite3"), b"ledger").unwrap();

        let outcome = migrate_legacy_entries(&legacy, &canonical).unwrap();

        assert_eq!(outcome, LegacyMigrationOutcome::NotNeeded);
        assert!(!stale_retirement.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_lock_wait_is_bounded() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            10
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let lock_path = root.join("lock");
        let first = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        let second = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        first.lock().unwrap();

        let error = lock_with_timeout(&second, std::time::Duration::from_millis(20)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        drop(first);
        std::fs::remove_dir_all(root).unwrap();
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
        assert!(combined.to_string().contains("in use"));
    }

    #[test]
    fn unverifiable_staging_residue_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "yap-app-data-migration-{}-{}",
            std::process::id(),
            11
        ));
        let legacy = root.join("legacy");
        let canonical = root.join("canonical");
        let stale_stage = canonical.join(".legacy-migration-stage-only-copy");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&stale_stage).unwrap();
        std::fs::write(stale_stage.join("jobs.sqlite3"), b"only copy").unwrap();

        let error = migrate_legacy_entries(&legacy, &canonical).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            std::fs::read(stale_stage.join("jobs.sqlite3")).unwrap(),
            b"only copy"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
