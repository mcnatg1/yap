use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

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
    if !legacy.exists() {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }
    if !legacy.is_dir() || is_link_or_reparse(legacy)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "legacy Yap data root is not a normal directory: {}",
                legacy.display()
            ),
        ));
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(legacy)? {
        let entry = entry?;
        if is_legacy_runtime_entry(&entry.file_name()) {
            entries.push((entry.path(), canonical.join(entry.file_name())));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    if entries.is_empty() {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }

    for (source, destination) in &entries {
        if is_link_or_reparse(source)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "legacy Yap runtime entry is a link or reparse point: {}",
                    source.display()
                ),
            ));
        }
        if destination.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "legacy Yap runtime entry conflicts with canonical app data: {}",
                    destination.display()
                ),
            ));
        }
    }

    fs::create_dir_all(canonical)?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(entries.len());
    for (source, destination) in &entries {
        if let Err(error) = fs::rename(source, destination) {
            let rollback_errors = moved
                .iter()
                .rev()
                .filter_map(|(moved_source, moved_destination)| {
                    fs::rename(moved_destination, moved_source).err()
                })
                .map(|rollback| rollback.to_string())
                .collect::<Vec<_>>();
            let detail = if rollback_errors.is_empty() {
                error.to_string()
            } else {
                format!(
                    "{error}; rollback also failed: {}",
                    rollback_errors.join("; ")
                )
            };
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "failed to migrate {} to {}: {detail}",
                    source.display(),
                    destination.display()
                ),
            ));
        }
        moved.push((source.clone(), destination.clone()));
    }

    Ok(LegacyMigrationOutcome::Migrated {
        entries: moved.len(),
    })
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

fn is_link_or_reparse(path: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Ok(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        Ok(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }
    #[cfg(not(windows))]
    Ok(false)
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
}
