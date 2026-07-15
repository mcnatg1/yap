use tauri::Manager;

use crate::{live, stt};

use super::completion::{
    append_error, apply_injection_result, run_completion_effects_with_mode, CompletionMode,
};

pub(crate) fn stop_live_from_app(app: &tauri::AppHandle) {
    let live = app.state::<live::LiveSessionState>();
    let live_runtime = app.state::<live::runtime::LiveRuntime>();
    let _ = stop_live_runtime(app.clone(), &live, &live_runtime);
}

pub(super) struct FinalizationOutcome {
    pub(super) view: live::state::LiveSessionView,
    pub(super) save_error: Option<String>,
}

pub(crate) fn stop_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
) -> live::state::LiveSessionView {
    live_runtime.cancel_pending_start();
    live_runtime.run_stop_lifecycle(|| finalize_live_runtime(app, live, live_runtime, None, None))
}

pub(crate) fn stop_live_runtime_after_crash(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    session: u64,
    message: &str,
) -> live::state::LiveSessionView {
    live_runtime.run_stop_lifecycle(|| {
        finalize_live_runtime(app, live, live_runtime, Some(session), Some(message))
    })
}

fn finalize_live_runtime(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    expected_session: Option<u64>,
    completion_error: Option<&str>,
) -> live::state::LiveSessionView {
    finalize_live_runtime_with_mode(
        app,
        live,
        live_runtime,
        expected_session,
        completion_error,
        CompletionMode::Normal,
    )
    .view
}

#[allow(clippy::too_many_arguments)]
pub(super) fn finalize_live_runtime_with_mode(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    live_runtime: &live::runtime::LiveRuntime,
    expected_session: Option<u64>,
    completion_error: Option<&str>,
    mode: CompletionMode,
) -> FinalizationOutcome {
    if expected_session.is_some_and(|session| !live_runtime.is_session_current(session)) {
        return FinalizationOutcome {
            view: live.snapshot(),
            save_error: None,
        };
    }
    let Some(saving) = live.try_begin_saving(live_runtime.is_active()) else {
        if let Some(message) = completion_error {
            if let Some(view) = live.update_if_saving(|view| {
                view.transcription_degraded = true;
                view.error = Some(append_error(view.error.take(), message));
            }) {
                live::events::emit_session(&app, &view);
                return FinalizationOutcome {
                    view,
                    save_error: None,
                };
            }
        }
        return FinalizationOutcome {
            view: live.snapshot(),
            save_error: None,
        };
    };
    let injection_target = (mode == CompletionMode::Normal)
        .then(live::injection::capture_target)
        .flatten();

    live::events::emit_session(&app, &saving);
    let finish_status = live_runtime.stop_stream();
    if finish_status.should_report() {
        stt::log_yap(&format!(
            "live stream stop completed with {finish_status:?}"
        ));
    }
    let before_stop = if finish_status.should_report() {
        live.mark_transcription_degraded()
    } else {
        live.snapshot()
    };
    let effects = run_completion_effects_with_mode(
        &before_stop,
        mode,
        |text| live.remember_completed_transcript(text),
        |text| live::injection::inject_text(&app, injection_target, text),
        || {
            let stop = live_runtime.finish_stop(finish_status);
            live::recordings::save_stop_result(&stop, &before_stop)
        },
    );
    apply_injection_result(live, effects.injection);
    if let Some(message) = completion_error {
        live.update(|view| {
            view.transcription_degraded = true;
            view.error = Some(append_error(view.error.take(), message));
        });
    }
    match effects.save {
        Ok(Some(saved)) => live::events::emit_saved(&app, &saved),
        Ok(None) => {}
        Err(error) => {
            stt::log_yap(&format!("live save failed: {error}"));
            live.update(|view| {
                let save_error = "Couldn't save this recording to Home.";
                view.error = Some(append_error(view.error.take(), save_error));
            });
            return finish_live_finalization(app, live, Some(error));
        }
    }
    finish_live_finalization(app, live, None)
}

fn finish_live_finalization(
    app: tauri::AppHandle,
    live: &live::LiveSessionState,
    save_error: Option<String>,
) -> FinalizationOutcome {
    let view = live.finish_saving();
    if view.error.is_some() {
        if let Err(error) = live::overlay_window::ensure_active(&app) {
            stt::log_yap(&format!("live overlay feedback show failed: {error}"));
        }
    } else if view.visibility == live::state::LiveOverlayVisibility::Enabled {
        if let Err(error) = live::overlay_window::ensure_idle(&app) {
            stt::log_yap(&format!("live overlay idle show failed: {error}"));
        }
    } else if let Some(window) = app.get_webview_window(live::overlay_window::WINDOW_LABEL) {
        let _ = window.hide();
    }
    live::events::emit_session(&app, &view);
    FinalizationOutcome { view, save_error }
}
