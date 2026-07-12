use std::path::{Path, PathBuf};

pub(crate) fn app_data_dir() -> PathBuf {
    let executable = std::env::current_exe().ok();
    app_data_dir_for_executable(|key| std::env::var(key).ok(), executable.as_deref())
}

pub(crate) fn app_data_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    let executable = std::env::current_exe().ok();
    app_data_dir_for_executable(env, executable.as_deref())
}

fn app_data_dir_for_executable<F>(env: F, executable: Option<&Path>) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(override_dir) = absolute_env_path(&env, "YAP_APP_DATA_DIR") {
        return override_dir;
    }
    let app_name = executable
        .and_then(Path::file_stem)
        .and_then(|stem| stem.to_str())
        .filter(|stem| stem.eq_ignore_ascii_case("yap-test"))
        .map_or("Yap", |_| "Yap.Test");
    if let Some(local) = absolute_env_path(&env, "LOCALAPPDATA") {
        return local.join(app_name);
    }
    if let Some(xdg) = absolute_env_path(&env, "XDG_DATA_HOME") {
        return xdg.join(app_name);
    }
    if let Some(home) = absolute_env_path(&env, "HOME") {
        return home.join(".local").join("share").join(app_name);
    }
    std::env::temp_dir().join(app_name)
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
            "LOCALAPPDATA" => Some(local.display().to_string()),
            _ => None,
        });

        assert_eq!(dir, override_dir);
    }

    #[test]
    fn app_data_dir_prefers_local_app_data() {
        let local = std::env::temp_dir().join("local-data");
        let xdg = std::env::temp_dir().join("xdg-data");
        let dir = app_data_dir_from(|key| match key {
            "LOCALAPPDATA" => Some(local.display().to_string()),
            "XDG_DATA_HOME" => Some(xdg.display().to_string()),
            _ => None,
        });

        assert_eq!(dir, local.join("Yap"));
    }

    #[test]
    fn test_binary_defaults_to_test_data_namespace() {
        let local = std::env::temp_dir().join("local-data");
        let executable = std::env::temp_dir().join("yap-test.exe");
        let dir = app_data_dir_for_executable(
            |key| (key == "LOCALAPPDATA").then(|| local.display().to_string()),
            Some(&executable),
        );

        assert_eq!(dir, local.join("Yap.Test"));
    }

    #[test]
    fn production_binary_keeps_production_data_namespace() {
        let local = std::env::temp_dir().join("local-data");
        let executable = std::env::temp_dir().join("yap-desktop.exe");
        let dir = app_data_dir_for_executable(
            |key| (key == "LOCALAPPDATA").then(|| local.display().to_string()),
            Some(&executable),
        );

        assert_eq!(dir, local.join("Yap"));
    }

    #[test]
    fn app_data_dir_uses_xdg_or_home_without_relative_paths() {
        let xdg = std::env::temp_dir().join("xdg-data");
        let home = std::env::temp_dir().join("home");
        assert_eq!(
            app_data_dir_from(|key| (key == "XDG_DATA_HOME").then(|| xdg.display().to_string())),
            xdg.join("Yap")
        );
        assert_eq!(
            app_data_dir_from(|key| (key == "HOME").then(|| home.display().to_string())),
            home.join(".local").join("share").join("Yap")
        );
        assert!(
            app_data_dir_from(|key| (key == "HOME").then(|| "relative-home".into())).is_absolute()
        );
    }

    #[test]
    fn absolute_env_path_rejects_relative_values() {
        let env = |key: &str| (key == "YAP_MODELS_DIR").then(|| "models".to_string());

        assert_eq!(absolute_env_path(&env, "YAP_MODELS_DIR"), None);
    }
}
