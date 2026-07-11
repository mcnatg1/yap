pub(crate) const MAIN_WINDOW_LABEL: &str = "main";
pub(crate) const LIVE_OVERLAY_WINDOW_LABEL: &str = "live-overlay";

pub(crate) fn is_main_window(label: &str) -> bool {
    label == MAIN_WINDOW_LABEL
}

pub(crate) fn is_main_or_overlay_window(label: &str) -> bool {
    is_main_window(label) || label == LIVE_OVERLAY_WINDOW_LABEL
}

fn forbidden_command_window_message() -> String {
    "Command is not available from this window.".into()
}

pub(crate) fn ensure_main(window: &tauri::WebviewWindow) -> Result<(), String> {
    is_main_window(window.label())
        .then_some(())
        .ok_or_else(forbidden_command_window_message)
}

pub(crate) fn ensure_main_or_overlay(window: &tauri::WebviewWindow) -> Result<(), String> {
    is_main_or_overlay_window(window.label())
        .then_some(())
        .ok_or_else(forbidden_command_window_message)
}

fn forbidden_stt_window() -> crate::stt::dispatch::SttCommandError {
    crate::stt::dispatch::SttCommandError {
        code: "UNAUTHORIZED_WINDOW".into(),
        message: forbidden_command_window_message(),
    }
}

pub(crate) fn ensure_main_stt(
    window: &tauri::WebviewWindow,
) -> Result<(), crate::stt::dispatch::SttCommandError> {
    is_main_window(window.label())
        .then_some(())
        .ok_or_else(forbidden_stt_window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_window_guards_keep_privileged_commands_main_only() {
        assert!(is_main_window(MAIN_WINDOW_LABEL));
        assert!(!is_main_window(LIVE_OVERLAY_WINDOW_LABEL));
        assert!(!is_main_window("settings"));

        assert!(is_main_or_overlay_window(MAIN_WINDOW_LABEL));
        assert!(is_main_or_overlay_window(LIVE_OVERLAY_WINDOW_LABEL));
        assert!(!is_main_or_overlay_window("settings"));
    }

    #[test]
    fn unauthorized_window_errors_keep_typed_and_untyped_contracts() {
        assert_eq!(
            forbidden_command_window_message(),
            "Command is not available from this window."
        );
        let error = forbidden_stt_window();

        assert_eq!(error.code, "UNAUTHORIZED_WINDOW");
        assert_eq!(error.message, "Command is not available from this window.");
    }
}
