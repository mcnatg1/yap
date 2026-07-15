use std::time::Instant;

use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::{authorization, live};

use super::{
    enrollment::{capture_physical_hotkey, HotkeyEnrollmentEpoch, HotkeyEnrollmentGate},
    kind::LiveHotkeyKind,
    registration::{
        apply_successful_hotkey_change, ensure_live_hotkey_idle, replace_hotkey_registration,
    },
};

pub(super) async fn record_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    gate: tauri::State<'_, HotkeyEnrollmentGate>,
    kind: LiveHotkeyKind,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    ensure_live_hotkey_idle(state.snapshot().status)?;
    let _lease = gate.try_begin()?;
    let confirmed = confirm_hotkey_enrollment(app.clone(), kind).await?;
    let Some(epoch) = HotkeyEnrollmentEpoch::arm(confirmed, kind, Instant::now()) else {
        return Ok(state.snapshot());
    };
    let captured = tauri::async_runtime::spawn_blocking(move || capture_physical_hotkey(epoch))
        .await
        .map_err(|error| format!("Shortcut recording worker failed: {error}"))??;
    let Some(hotkey) = captured else {
        return Ok(state.snapshot());
    };
    change_hotkey(window, app, state, kind, Some(hotkey))
}

pub(super) fn change_hotkey(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: tauri::State<'_, live::LiveSessionState>,
    kind: LiveHotkeyKind,
    hotkey: Option<String>,
) -> Result<live::state::LiveSessionView, String> {
    authorization::ensure_main(&window)?;
    ensure_live_hotkey_idle(state.snapshot().status)?;

    let snapshot = state.snapshot();
    let requested_value = hotkey
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let (next_value, next) = if requested_value.is_empty() {
        (String::new(), None)
    } else {
        let normalized = live::hotkeys::normalize_hotkey_for(&requested_value, kind.purpose())?;
        let shortcut = live::hotkeys::parse_hotkey_for(&normalized, kind.purpose())?;
        (normalized, Some(shortcut))
    };
    if live::hotkeys::configured_hotkeys_match(kind.conflicting(&snapshot), &next_value) {
        return Err(kind.conflict_message().into());
    }
    if kind.current(&snapshot) == next_value {
        return Ok(snapshot);
    }

    let previous = live::hotkeys::parse_hotkey_for(kind.current(&snapshot), kind.purpose()).ok();
    let mut prospective = snapshot.clone();
    kind.update(&mut prospective, next_value.clone());
    replace_hotkey_registration(
        previous,
        next,
        |shortcut| {
            app.global_shortcut()
                .unregister(shortcut)
                .map_err(|error| format!("Failed to unregister previous shortcut: {error}"))
        },
        |shortcut| {
            app.global_shortcut()
                .register(shortcut)
                .map_err(|error| error.to_string())
        },
        || live::settings::save_view(&prospective),
    )?;
    live::shortcut_runtime::reset(&app);

    let recovered_startup_failure = state.take_startup_shortcut_failure(kind.is_paste());
    let view = state.update(|view| {
        apply_successful_hotkey_change(view, kind, next_value, recovered_startup_failure);
    });
    live::events::emit_session(&app, &view);
    Ok(view)
}

async fn confirm_hotkey_enrollment(
    app: tauri::AppHandle,
    kind: LiveHotkeyKind,
) -> Result<bool, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let label = kind.label();
    let modifier_count = kind.required_modifier_count();
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "Record a new {label} shortcut?\n\nAfter choosing Record, hold at least {modifier_count} modifier keys plus one key, then release the entire chord. Yap listens for only this one physical chord for 15 seconds. Press Escape to cancel."
            ))
            .title("Record physical shortcut")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Record".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("Could not show shortcut confirmation: {error}"))
}
