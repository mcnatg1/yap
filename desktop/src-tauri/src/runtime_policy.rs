use crate::{runtime, stt};

pub(crate) fn current_setup_status() -> SetupStatus {
    let fallback_enabled = stt::settings::local_fallback_enabled();
    let model_installed = matches!(
        stt::nemotron::model_status(fallback_enabled).status,
        stt::nemotron::FallbackModelStatus::Ready | stt::nemotron::FallbackModelStatus::Disabled
    );
    let (setup_state, engine_ready, engine_status) =
        compose_engine_status(stt::nemotron::local_fallback_readiness());
    SetupStatus {
        model: stt::nemotron::MODEL_LABEL.into(),
        root: stt::nemotron::root_dir().display().to_string(),
        engine_ready,
        engine_binary_status: "Built in".into(),
        model_installed,
        fallback_enabled,
        engine_status,
        setup_state,
    }
}

fn compose_engine_status(
    availability: Result<(), stt::error::SttError>,
) -> (runtime::state::SetupState, bool, String) {
    match availability {
        Ok(()) => (
            runtime::state::SetupState::FallbackReady,
            true,
            "Transcription engine ready".into(),
        ),
        Err(stt::error::SttError::FallbackDisabled) => (
            runtime::state::SetupState::FallbackDisabled,
            false,
            "Local fallback disabled".into(),
        ),
        Err(stt::error::SttError::ModelMissing) => (
            runtime::state::SetupState::FallbackMissing,
            false,
            "Local fallback model missing".into(),
        ),
        Err(stt::error::SttError::ModelCorrupt) => (
            runtime::state::SetupState::SetupError,
            false,
            stt::error::SttError::ModelCorrupt.user_message().into(),
        ),
        Err(_) => (
            runtime::state::SetupState::SetupError,
            false,
            "Local fallback needs attention.".into(),
        ),
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetupStatus {
    model: String,
    root: String,
    engine_ready: bool,
    engine_binary_status: String,
    model_installed: bool,
    fallback_enabled: bool,
    engine_status: String,
    #[serde(skip_serializing)]
    setup_state: runtime::state::SetupState,
}

impl SetupStatus {
    pub(crate) fn runtime_setup_state(&self) -> runtime::state::SetupState {
        self.setup_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_status_serializes_for_frontend() {
        let value = serde_json::to_value(SetupStatus {
            model: "model".into(),
            root: "root".into(),
            engine_ready: true,
            engine_binary_status: "Built in".into(),
            model_installed: true,
            fallback_enabled: true,
            engine_status: "Transcription engine ready".into(),
            setup_state: runtime::state::SetupState::FallbackReady,
        })
        .unwrap();

        assert_eq!(value["engineReady"], true);
        assert_eq!(value["engineBinaryStatus"], "Built in");
        assert_eq!(value["modelInstalled"], true);
        assert_eq!(value["fallbackEnabled"], true);
        assert_eq!(value["engineStatus"], "Transcription engine ready");
        assert!(value.get("python_ready").is_none());
    }

    #[test]
    fn disabled_status_wins() {
        assert_eq!(
            compose_engine_status(Err(stt::error::SttError::FallbackDisabled)),
            (
                runtime::state::SetupState::FallbackDisabled,
                false,
                "Local fallback disabled".into()
            )
        );
    }

    #[test]
    fn runtime_setup_state_preserves_model_failures() {
        let missing_model = SetupStatus {
            model: "model".into(),
            root: "root".into(),
            engine_ready: false,
            engine_binary_status: "Built in".into(),
            model_installed: false,
            fallback_enabled: true,
            engine_status: "Setup".into(),
            setup_state: runtime::state::SetupState::FallbackMissing,
        };

        assert_eq!(
            missing_model.runtime_setup_state(),
            runtime::state::SetupState::FallbackMissing
        );
    }

    #[test]
    fn corrupt_status_maps_to_setup_error() {
        assert_eq!(
            compose_engine_status(Err(stt::error::SttError::ModelCorrupt)),
            (
                runtime::state::SetupState::SetupError,
                false,
                stt::error::SttError::ModelCorrupt.user_message().into()
            )
        );
    }
}
