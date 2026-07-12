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
    let saved = config::save(&settings).map_err(|error| error.to_string())?;
    generation.invalidate();
    Ok(saved)
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
}
