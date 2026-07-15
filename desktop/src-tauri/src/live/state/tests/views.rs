use super::super::*;
use crate::{live::settings::LiveSettings, runtime};

#[test]
fn live_start_uses_local_fallback_without_server() {
    assert_eq!(
        live_route_for(runtime::state::SetupState::FallbackReady, false),
        LiveRoute::LocalFallback
    );
}

#[test]
fn live_state_restores_paste_hotkey_settings() {
    let view = LiveSessionView::from_settings(&LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: Some("Ctrl+Shift+V".into()),
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });

    assert_eq!(view.paste_hotkey, "Ctrl+Shift+V");
}

#[test]
fn live_state_rejects_a_paste_hotkey_that_conflicts_with_dictation() {
    let view = LiveSessionView::from_settings(&LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: Some("Ctrl+Shift+Space".into()),
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });

    assert_eq!(view.hotkey, "Ctrl+Shift+Space");
    assert_eq!(view.paste_hotkey, "");
}

#[test]
fn live_start_blocks_without_any_route() {
    assert_eq!(
        live_route_for(runtime::state::SetupState::FallbackMissing, false),
        LiveRoute::Blocked
    );
}

#[test]
fn level_view_does_not_serialize_transcript_text() {
    let mut view = LiveSessionView::from_settings(&LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: None,
        capture_mode: LiveCaptureMode::PushToTalk,
        input_device_id: None,
    });
    view.partial_text = Some("large partial".into());
    view.final_text = Some("large final".into());
    view.level = Some(0.5);
    view.status = LiveSessionStatus::Speaking;

    let payload = serde_json::to_value(LiveLevelView::from(&view)).unwrap();

    assert_eq!(
        payload.get("level").and_then(serde_json::Value::as_f64),
        Some(0.5)
    );
    assert!(payload.get("status").is_none());
    assert!(payload.get("partialText").is_none());
    assert!(payload.get("finalText").is_none());
}

#[test]
fn overlay_view_exposes_only_rendering_state() {
    let mut view = LiveSessionView::from_settings(&LiveSettings {
        overlay_enabled: true,
        hotkey: Some("Ctrl+Shift+Space".into()),
        paste_hotkey: Some("Ctrl+Shift+Alt+V".into()),
        capture_mode: LiveCaptureMode::Toggle,
        input_device_id: Some("private-device-id".into()),
    });
    view.partial_text = Some("private partial transcript".into());
    view.final_text = Some("private final transcript".into());
    view.input_device_label = Some("Private microphone".into());
    view.status = LiveSessionStatus::Speaking;

    let payload = serde_json::to_value(LiveOverlayView::from(&view)).unwrap();

    assert_eq!(
        payload.get("hasFinalText"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(
        payload
            .get("captureMode")
            .and_then(serde_json::Value::as_str),
        Some("toggle")
    );
    for forbidden in [
        "partialText",
        "finalText",
        "hotkey",
        "pasteHotkey",
        "inputDeviceId",
        "inputDeviceLabel",
        "route",
        "transcriptionDegraded",
    ] {
        assert!(
            payload.get(forbidden).is_none(),
            "unexpected overlay field: {forbidden}"
        );
    }
}
