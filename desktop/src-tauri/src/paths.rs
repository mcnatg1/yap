use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

const PRODUCTION_IDENTIFIER: &str = "com.mcnatg1.yap";
const LEGACY_APP_NAME: &str = "Yap";

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
    if initial_entries.is_empty() {
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
    lock_file.lock()?;
    migrate_legacy_entries_locked(legacy, canonical)
}

fn migrate_legacy_entries_locked(
    legacy: &Path,
    canonical: &Path,
) -> io::Result<LegacyMigrationOutcome> {
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
        Some(create_unique_directory(
            canonical,
            ".legacy-migration-stage",
        )?)
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
            let _ = fs::remove_dir_all(staging);
            return Err(error);
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
            let _ = fs::remove_dir_all(staging);
            return Err(error);
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

    let retirement = create_unique_directory(legacy, ".yap-runtime-migrated")?;
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
    let _ = fs::remove_dir_all(&retirement);

    Ok(LegacyMigrationOutcome::Migrated {
        entries: entries.len(),
    })
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
}
