use std::{io, path::PathBuf};

mod legacy_migration;

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
    legacy_migration::migrate_legacy_entries(&legacy, &canonical_root.join(PRODUCTION_IDENTIFIER))
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
        let dir = app_data_dir_from(|key| match key {
            "YAP_APP_DATA_DIR" => Some(override_dir.display().to_string()),
            _ => None,
        });

        assert_eq!(dir, override_dir);
    }

    #[test]
    fn app_data_dir_uses_tauri_data_root_and_identifier() {
        let data_root = std::env::temp_dir().join("yap-roaming-data");
        let dir = app_data_dir_from_root(|_| None, Some(data_root.clone()));

        assert_eq!(dir, data_root.join("com.mcnatg1.yap"));
    }

    #[test]
    fn app_data_dir_keeps_production_data_namespace() {
        let local = std::env::temp_dir().join("yap-local-data");
        let dir = app_data_dir_from(|key| match key {
            "APPDATA" => Some(local.display().to_string()),
            _ => None,
        });

        assert_eq!(dir, local.join("com.mcnatg1.yap"));
    }

    #[test]
    #[should_panic(expected = "Tauri app-data root is unavailable")]
    fn missing_tauri_data_root_fails_closed() {
        app_data_dir_from_root(|_| None, None);
    }

    #[test]
    fn absolute_env_path_rejects_relative_values() {
        assert!(absolute_env_path(&|_| Some("relative/path".into()), "HOME").is_none());
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
}
