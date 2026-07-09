use std::path::PathBuf;

pub(crate) fn app_data_dir() -> PathBuf {
    app_data_dir_from(|key| std::env::var(key).ok())
}

pub(crate) fn app_data_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(local) = absolute_env_path(&env, "LOCALAPPDATA") {
        return local.join("Yap");
    }
    if let Some(xdg) = absolute_env_path(&env, "XDG_DATA_HOME") {
        return xdg.join("Yap");
    }
    if let Some(home) = absolute_env_path(&env, "HOME") {
        return home.join(".local").join("share").join("Yap");
    }
    std::env::temp_dir().join("Yap")
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
