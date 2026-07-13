use std::path::PathBuf;

const PRODUCTION_IDENTIFIER: &str = "com.mcnatg1.yap";

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
        .unwrap_or_else(std::env::temp_dir)
        .join(PRODUCTION_IDENTIFIER)
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
    fn missing_tauri_data_root_falls_back_to_temp_with_identifier() {
        let dir = app_data_dir_from_root(|_| None, None);

        assert_eq!(dir, std::env::temp_dir().join("com.mcnatg1.yap"));
    }

    #[test]
    fn absolute_env_path_rejects_relative_values() {
        let env = |key: &str| (key == "YAP_MODELS_DIR").then(|| "models".to_string());

        assert_eq!(absolute_env_path(&env, "YAP_MODELS_DIR"), None);
    }
}
