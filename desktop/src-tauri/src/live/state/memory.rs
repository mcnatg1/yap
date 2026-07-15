use std::sync::Mutex;

#[derive(Default)]
pub(super) struct SessionMemory {
    last_completed_transcript: Mutex<Option<String>>,
    startup_shortcut_failures: Mutex<StartupShortcutFailures>,
}

#[derive(Default)]
struct StartupShortcutFailures {
    dictation: bool,
    paste_last: bool,
}

impl SessionMemory {
    pub(super) fn mark_startup_shortcut_failure(&self, is_paste: bool) {
        let mut failures = self
            .startup_shortcut_failures
            .lock()
            .expect("live startup shortcut state poisoned");
        if is_paste {
            failures.paste_last = true;
        } else {
            failures.dictation = true;
        }
    }

    pub(super) fn take_startup_shortcut_failure(&self, is_paste: bool) -> bool {
        let mut failures = self
            .startup_shortcut_failures
            .lock()
            .expect("live startup shortcut state poisoned");
        let failed = if is_paste {
            &mut failures.paste_last
        } else {
            &mut failures.dictation
        };
        std::mem::take(failed)
    }

    pub(super) fn clear_startup_shortcut_failure(&self, is_paste: bool) {
        let _ = self.take_startup_shortcut_failure(is_paste);
    }

    pub(super) fn last_completed_transcript(&self) -> Option<String> {
        self.last_completed_transcript
            .lock()
            .expect("live completed transcript state poisoned")
            .clone()
    }

    pub(super) fn remember_completed_transcript(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        *self
            .last_completed_transcript
            .lock()
            .expect("live completed transcript state poisoned") = Some(text.to_string());
    }
}
