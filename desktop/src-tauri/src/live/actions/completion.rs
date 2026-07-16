use tauri::Manager;

use crate::{live, stt};

pub(super) const INJECTION_COPIED_ERROR: &str =
    "Couldn't insert text here. Transcript copied; press Ctrl+V.";
const INJECTION_FAILED_ERROR: &str = "Couldn't insert or copy this transcript.";

pub(super) fn append_error(existing: Option<String>, message: &str) -> String {
    match existing {
        Some(existing) if existing.contains(message) => existing,
        Some(existing) => format!("{existing} {message}"),
        None => message.into(),
    }
}

fn without_injection_feedback(error: Option<&str>) -> Option<String> {
    let error = error?;
    if !error.contains(INJECTION_COPIED_ERROR) && !error.contains(INJECTION_FAILED_ERROR) {
        return Some(error.to_string());
    }
    let cleaned = error
        .replace(INJECTION_COPIED_ERROR, "")
        .replace(INJECTION_FAILED_ERROR, "");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cleaned.is_empty()).then_some(cleaned)
}

pub(super) struct CompletionEffects<I, S> {
    pub(super) injection: Result<Option<I>, String>,
    pub(super) save: Result<S, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CompletionMode {
    Normal,
    Quit,
}

#[cfg(test)]
pub(super) fn run_completion_effects_with<I, S>(
    view: &live::state::LiveSessionView,
    remember: impl FnOnce(&str),
    inject: impl FnOnce(&str) -> Result<I, String>,
    save: impl FnOnce() -> Result<S, String>,
) -> CompletionEffects<I, S> {
    run_completion_effects_with_mode(view, CompletionMode::Normal, remember, inject, save)
}

pub(super) fn run_completion_effects_with_mode<I, S>(
    view: &live::state::LiveSessionView,
    mode: CompletionMode,
    remember: impl FnOnce(&str),
    inject: impl FnOnce(&str) -> Result<I, String>,
    save: impl FnOnce() -> Result<S, String>,
) -> CompletionEffects<I, S> {
    let text = live::recordings::completed_transcript_text(view);
    if let Some(text) = text.as_deref() {
        remember(text);
    }
    let injection = match (mode, text) {
        (CompletionMode::Normal, Some(text)) => inject(&text).map(Some),
        (CompletionMode::Normal | CompletionMode::Quit, None) | (CompletionMode::Quit, Some(_)) => {
            Ok(None)
        }
    };
    CompletionEffects {
        injection,
        save: save(),
    }
}

pub(crate) fn inject_last_live_transcript(
    app: &tauri::AppHandle,
    target: Option<live::injection::InjectionTarget>,
) {
    let live = app.state::<live::LiveSessionState>();
    let result = match live.last_completed_transcript() {
        Some(text) => live::injection::inject_text(app, target, &text).map(Some),
        None => Ok(None),
    };
    let view = apply_injection_result(&live, result);
    if view.error.is_some() {
        if let Err(error) = live::overlay_window::ensure_active(app) {
            stt::log_yap(&format!("live paste feedback show failed: {error}"));
        }
    } else if view.visibility == live::state::LiveOverlayVisibility::Enabled {
        if let Err(error) = live::overlay_window::ensure_idle(app) {
            stt::log_yap(&format!("live paste idle show failed: {error}"));
        }
    } else if let Some(window) = app.get_webview_window(live::overlay_window::WINDOW_LABEL) {
        let _ = window.hide();
    }
    live::events::emit_session(app, &view);
}

pub(super) fn apply_injection_result(
    live: &live::LiveSessionState,
    result: Result<Option<live::injection::InjectionOutcome>, String>,
) -> live::state::LiveSessionView {
    match result {
        Ok(Some(live::injection::InjectionOutcome::Injected)) => live.update(|view| {
            view.error = without_injection_feedback(view.error.as_deref());
        }),
        Ok(Some(live::injection::InjectionOutcome::CopiedOnly(reason))) => {
            stt::log_yap(&format!("live injection copied fallback: {reason}"));
            live.update(|view| {
                let existing = without_injection_feedback(view.error.as_deref());
                view.error = Some(append_error(existing, INJECTION_COPIED_ERROR));
            })
        }
        Ok(Some(live::injection::InjectionOutcome::Ignored)) | Ok(None) => live.snapshot(),
        Err(error) => {
            stt::log_yap(&format!("live transcript injection failed: {error}"));
            live.update(|view| {
                let existing = without_injection_feedback(view.error.as_deref());
                view.error = Some(append_error(existing, INJECTION_FAILED_ERROR));
            })
        }
    }
}
