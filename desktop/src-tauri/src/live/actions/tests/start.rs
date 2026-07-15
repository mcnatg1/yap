use super::super::start::block_for_setup;
use crate::{
    live::{settings::LiveSettings, state::LiveSessionState},
    runtime::state::SetupState,
};

#[test]
fn start_live_setup_missing_blocks_without_claiming_server() {
    let live = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: None,
        capture_mode: crate::live::state::LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });

    let view = block_for_setup(&live, SetupState::FallbackMissing);

    assert_eq!(view.status, crate::live::state::LiveSessionStatus::Blocked);
    assert_eq!(view.route, crate::live::state::LiveRoute::Blocked);
    assert_eq!(view.error.as_deref(), Some("Local fallback is not ready."));
}
