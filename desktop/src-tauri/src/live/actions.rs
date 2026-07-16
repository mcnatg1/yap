mod completion;
mod quit;
mod start;
mod stop;

pub(crate) use completion::inject_last_live_transcript;
pub(crate) use quit::{quit_from_app, show_main_window, QuitCoordinator};
pub(crate) use start::{
    configured_hotkey_matches_shortcut, handle_live_shortcut_action, start_live_from_app,
    start_live_runtime, warm_on_intent,
};
pub(crate) use stop::{stop_live_from_app, stop_live_runtime, stop_live_runtime_after_crash};

#[cfg(test)]
mod tests;
