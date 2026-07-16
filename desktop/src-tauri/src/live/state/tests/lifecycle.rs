use super::super::*;
use crate::{live::settings::LiveSettings, runtime};

#[test]
fn capture_start_cannot_overwrite_a_saving_lease() {
    let state = LiveSessionState::new(LiveSettings::default());
    state.update(|view| view.status = LiveSessionStatus::Armed);
    state.try_begin_saving(false).unwrap();

    assert!(state.try_begin_listening_from_armed().is_none());
    assert_eq!(state.snapshot().status, LiveSessionStatus::Saving);
}

#[test]
fn route_loss_downgrades_server_to_fallback() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: None,
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    state.update(|view| {
        view.route = LiveRoute::ServerLive;
        view.status = LiveSessionStatus::Listening;
    });

    let view = state.route_loss(true);

    assert_eq!(view.route, LiveRoute::LocalFallback);
    assert_eq!(
        view.error.as_deref(),
        Some("Server unavailable. Using local fallback.")
    );
}

#[test]
fn toggle_stops_active_capture() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: None,
        capture_mode: LiveCaptureMode::Toggle,
        input_device_id: None,
    });
    state.start(runtime::state::SetupState::FallbackReady, false);

    let view = state.toggle(runtime::state::SetupState::FallbackReady, false);

    assert_eq!(view.status, LiveSessionStatus::Idle);
    assert_eq!(view.route, LiveRoute::None);
}

#[test]
fn start_arms_without_claiming_mic_capture() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: None,
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });

    let view = state.start(runtime::state::SetupState::FallbackReady, false);

    assert_eq!(view.status, LiveSessionStatus::Armed);
    assert!(!is_live_capture_active(view.status));
}

#[test]
fn stop_preserves_final_text() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: None,
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    state.update(|view| view.status = LiveSessionStatus::Speaking);
    state.update_final("hello.");

    let view = state.stop();

    assert_eq!(view.final_text.as_deref(), Some("hello."));
}

#[test]
fn stream_crash_blocks_without_losing_final_text() {
    let state = LiveSessionState::new(LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: None,
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    state.update(|view| view.status = LiveSessionStatus::Speaking);
    state.update_final("kept.");

    let view = state.block_with_error("Live stream stopped.");

    assert_eq!(view.status, LiveSessionStatus::Blocked);
    assert_eq!(view.final_text.as_deref(), Some("kept."));
}
