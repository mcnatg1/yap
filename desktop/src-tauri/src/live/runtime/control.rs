use std::{sync::atomic::Ordering, time::Duration};

use super::{
    log_worker_shutdown_errors, LiveRuntime, LiveStreamEngine, ModelMutationLease, StartIntent,
};

impl LiveRuntime {
    pub fn is_active(&self) -> bool {
        self.inner
            .lock()
            .expect("live runtime poisoned")
            .is_capturing()
    }

    pub(crate) fn capture_start_intent(&self) -> StartIntent {
        StartIntent(self.start_generation.load(Ordering::Acquire))
    }

    pub(crate) fn start_intent_is_current(&self, intent: StartIntent) -> bool {
        self.start_generation.load(Ordering::Acquire) == intent.0
    }

    pub(crate) fn cancel_pending_start(&self) {
        self.start_generation.fetch_add(1, Ordering::AcqRel);
        self.model_warmup.cancel_loading();
    }

    pub(crate) fn run_start_lifecycle<T>(
        &self,
        intent: StartIntent,
        run: impl FnOnce() -> T,
    ) -> Option<T> {
        if self.model_mutation_active.load(Ordering::Acquire) {
            return None;
        }
        let _operation = self.transition.begin_start();
        self.start_intent_is_current(intent).then(run)
    }

    pub(crate) fn run_stop_lifecycle<T>(&self, run: impl FnOnce() -> T) -> T {
        let _operation = self.transition.begin_stop();
        run()
    }

    pub(crate) fn begin_model_mutation(&self) -> Result<ModelMutationLease, String> {
        self.cancel_pending_start();
        let operation = self.transition.begin_stop_owned();
        self.model_mutation_active.store(true, Ordering::Release);
        let lease = ModelMutationLease {
            runtime: self.clone(),
            _operation: operation,
        };

        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if inner.is_capturing() {
            return Err("Stop live before changing local fallback.".to_string());
        }
        inner.retire_stream();
        drop(inner);
        self.model_warmup.clear_idle()?;
        Ok(lease)
    }

    pub fn request_warm(&self, _app: tauri::AppHandle) -> Result<bool, String> {
        if self.model_mutation_active.load(Ordering::Acquire) {
            return Ok(false);
        }
        if self
            .inner
            .lock()
            .expect("live runtime poisoned")
            .has_running_stream()
        {
            return Ok(false);
        }

        self.request_model_warmup()
    }

    pub(super) fn request_model_warmup(&self) -> Result<bool, String> {
        self.model_warmup.request("live-model-warmup", || {
            LiveStreamEngine::new().map_err(|error| error.user_message().to_string())
        })
    }

    pub fn unload_if_idle(&self, threshold: Duration) {
        self.run_stop_lifecycle(|| {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if inner.is_idle_for(threshold) {
                inner.retire_stream();
                drop(inner);
                let _ = self.model_warmup.clear_idle();
            }
        });
    }

    pub fn shutdown(&self) {
        self.cancel_pending_start();
        self.run_stop_lifecycle(|| {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let (shutdown_errors, _) = inner.stop_capture();
            inner.retire_stream();
            self.active_session.store(0, Ordering::SeqCst);
            drop(inner);
            let _ = self.model_warmup.clear_idle();
            let _ = self.finalize_recording();
            log_worker_shutdown_errors(shutdown_errors);
        });
    }
}
