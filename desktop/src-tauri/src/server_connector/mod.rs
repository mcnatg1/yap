pub mod config;

#[derive(Default)]
pub(crate) struct ConnectorGeneration(std::sync::atomic::AtomicU64);

impl ConnectorGeneration {
    #[cfg(test)]
    pub(crate) fn current(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn invalidate(&self) -> u64 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1
    }
}

#[tauri::command]
pub(crate) fn server_settings(
    window: tauri::WebviewWindow,
) -> Result<config::ServerSettings, String> {
    crate::authorization::ensure_main(&window)?;
    config::load().map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn set_server_settings(
    window: tauri::WebviewWindow,
    generation: tauri::State<'_, ConnectorGeneration>,
    settings: config::ServerSettings,
) -> Result<config::ServerSettings, String> {
    crate::authorization::ensure_main(&window)?;
    finish_settings_save(&generation, config::save(&settings))
}

fn finish_settings_save(
    generation: &ConnectorGeneration,
    result: Result<config::ServerSettings, config::ConfigError>,
) -> Result<config::ServerSettings, String> {
    match result {
        Ok(saved) => {
            generation.invalidate();
            Ok(saved)
        }
        Err(error) => {
            if error.settings_were_published() {
                generation.invalidate();
            }
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_changes_advance_the_connector_generation() {
        let generation = ConnectorGeneration::default();

        assert_eq!(generation.current(), 0);
        assert_eq!(generation.invalidate(), 1);
        assert_eq!(generation.current(), 1);
    }

    #[test]
    fn post_publication_durability_failure_invalidates_generation_and_reports_visible_change() {
        let dir = std::env::temp_dir().join(format!(
            "yap-server-settings-post-publish-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server-settings.json");
        let settings = config::ServerSettings {
            schema_version: config::CURRENT_SCHEMA_VERSION,
            enabled: true,
            base_url: Some("https://visible.example".into()),
        };
        let save_result = config::save_to_path_with_hooks(
            &settings,
            &path,
            false,
            || Ok(()),
            || Ok(()),
            |_, _| Ok(()),
            |_| Err(std::io::Error::other("injected parent fsync failure")),
        );
        let generation = ConnectorGeneration::default();

        let error = finish_settings_save(&generation, save_result).unwrap_err();

        assert_eq!(generation.current(), 1);
        assert!(error.starts_with("Server settings changed, but durability confirmation failed:"));
        assert_eq!(config::load_from_path(&path, false).unwrap(), settings);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pre_publication_failure_does_not_invalidate_generation() {
        let generation = ConnectorGeneration::default();
        let result = Err(config::ConfigError::SaveIo(std::io::Error::other(
            "injected staging failure",
        )));

        let error = finish_settings_save(&generation, result).unwrap_err();

        assert_eq!(generation.current(), 0);
        assert!(error.starts_with("Could not save server settings:"));
    }
}
