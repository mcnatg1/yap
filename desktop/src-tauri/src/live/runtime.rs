#[cfg(test)]
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use tauri::Manager;

#[cfg(test)]
use crate::audio::coordinator::{bounded_sink, SinkKind, RECORDING_QUEUE_CAPACITY};
use crate::audio::recording::RecordingFinalizeResult;
#[cfg(test)]
use crate::audio::recording::RecordingSinkHandle;
use crate::audio::session::SessionId;

use super::state::LiveSessionState;
use super::stream::LiveStreamEngine;
#[cfg(test)]
use super::stream::{self, StreamMessage};

mod asr_adapter;
mod capture_worker;
mod finalization;
mod level_channel;
mod lifecycle_gate;
mod local_start;
mod resources;
mod session_identity;
mod stream_session;
mod warmup;
mod worker;

#[cfg(test)]
use asr_adapter::set_reaper_spawn_failure_for_test;
#[cfg(test)]
use asr_adapter::{AdapterDrainStatus, PendingAsrAdapter, SessionAsrAdapter};
#[cfg(test)]
use capture_worker::*;
use finalization::{RecordingFinalization, StopCompletion};
#[cfg(test)]
use level_channel::{level_channel, publish_level};
use lifecycle_gate::{LifecycleGate, OwnedLifecycleOperation};
pub(crate) use local_start::LocalCaptureStart;
use resources::LiveRuntimeResources;
#[cfg(test)]
use resources::{
    capture_install_is_current as should_install_capture, LiveRuntimeResources as LiveRuntimeInner,
};
use session_identity::{active_session_matches, CRASH_CLAIM_BIT};
#[cfg(test)]
use stream_session::should_accept_stream_samples;
#[cfg(test)]
use stream_session::SessionStream;
pub use stream_session::StreamFinishStatus;
use stream_session::StreamFinisher;
use warmup::SharedWarmup;

#[derive(Clone)]
pub struct LiveRuntime {
    inner: Arc<Mutex<LiveRuntimeResources>>,
    active_session: Arc<AtomicU64>,
    start_generation: Arc<AtomicU64>,
    recording_finalization: Arc<RecordingFinalization>,
    stop_completion: Arc<StopCompletion<LiveStopResult>>,
    transition: Arc<LifecycleGate>,
    model_warmup: Arc<SharedWarmup<LiveStreamEngine>>,
    model_mutation_active: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
pub(crate) struct StartIntent(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveStopResult {
    pub stream: StreamFinishStatus,
    pub recording: Result<Option<RecordingFinalizeResult>, String>,
}

/// Excludes live start/stop work while installed model files or enablement change.
pub(crate) struct ModelMutationLease {
    runtime: LiveRuntime,
    _operation: OwnedLifecycleOperation,
}

pub(crate) struct LiveStartFailure {
    session: u64,
    message: String,
}

impl LiveStartFailure {
    fn new(session: u64, message: String) -> Self {
        Self { session, message }
    }
}

impl Drop for ModelMutationLease {
    fn drop(&mut self) {
        // A start requested during a long install must not run unexpectedly afterward.
        self.runtime.cancel_pending_start();
        self.runtime
            .model_mutation_active
            .store(false, Ordering::Release);
    }
}

impl LiveRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LiveRuntimeResources::new())),
            active_session: Arc::new(AtomicU64::new(0)),
            start_generation: Arc::new(AtomicU64::new(0)),
            recording_finalization: Arc::new(RecordingFinalization::new()),
            stop_completion: Arc::new(StopCompletion::new()),
            transition: Arc::new(LifecycleGate::new()),
            model_warmup: Arc::new(SharedWarmup::new()),
            model_mutation_active: Arc::new(AtomicBool::new(false)),
        }
    }

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

    fn request_model_warmup(&self) -> Result<bool, String> {
        self.model_warmup.request("live-model-warmup", || {
            LiveStreamEngine::new().map_err(|error| error.user_message().to_string())
        })
    }

    pub fn stop(&self) -> LiveStopResult {
        self.cancel_pending_start();
        self.run_stop_lifecycle(|| {
            let stream = self.stop_stream();
            self.finish_stop(stream)
        })
    }

    pub(crate) fn stop_stream(&self) -> StreamFinishStatus {
        let (finisher, adapter_status, shutdown_errors) = {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let (shutdown_errors, adapter_status) = inner.stop_capture();
            (
                adapter_status
                    .is_none()
                    .then(|| inner.stream_finisher())
                    .flatten(),
                adapter_status,
                shutdown_errors,
            )
        };
        log_worker_shutdown_errors(shutdown_errors);
        let finish_status = adapter_status.unwrap_or_else(|| {
            finisher
                .as_ref()
                .map(StreamFinisher::finish_session)
                .unwrap_or(StreamFinishStatus::NoStream)
        });
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if finish_status.should_retire_stream() {
            inner.retire_stream_detached_reader();
        }
        self.active_session.store(0, Ordering::SeqCst);
        inner.mark_used();
        finish_status
    }

    pub(crate) fn finish_stop(&self, stream: StreamFinishStatus) -> LiveStopResult {
        self.stop_completion.complete_with(|| LiveStopResult {
            stream,
            recording: self.finalize_recording(),
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

    pub(crate) fn finalize_recording(&self) -> Result<Option<RecordingFinalizeResult>, String> {
        self.recording_finalization
            .finalize_with(|| match self.inner.lock() {
                Ok(mut inner) => {
                    let recording = inner.take_recording();
                    let session_id = recording
                        .as_ref()
                        .map(|recording| recording.session_id().clone());
                    (
                        recording.map(|recording| recording.finalize()).transpose(),
                        session_id,
                    )
                }
                Err(_) => (Err("live runtime became unavailable".into()), None),
            })
    }

    pub(crate) fn recording_finalization_failure(&self) -> Option<(SessionId, String)> {
        self.recording_finalization.failure()
    }

    fn ensure_recording_ready_to_start(&self) -> Result<(), String> {
        let prior_recording = self
            .inner
            .lock()
            .map_err(|_| "live runtime became unavailable")?
            .recording_is_present();
        if prior_recording {
            return Err("Previous live recording must be finalized before starting again.".into());
        }
        self.recording_finalization.prepare_for_new_recording()?;
        self.stop_completion.reset()
    }

    #[cfg(test)]
    pub(crate) fn install_unavailable_recording_for_test(&self, session_id: SessionId) {
        let (sink, _receiver) = bounded_sink(SinkKind::Recording, 1);
        self.inner.lock().unwrap().set_recording_for_test(
            RecordingSinkHandle::spawn_unavailable_for_test(sink, session_id),
        );
    }

    #[cfg(test)]
    pub(crate) fn install_panicking_recording_for_test(&self, session_id: SessionId) {
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        self.inner.lock().unwrap().set_recording_for_test(
            RecordingSinkHandle::spawn_panicking_for_test(sink, receiver, session_id),
        );
    }

    pub fn handle_stream_crash(&self, app: tauri::AppHandle, session: u64, message: &str) {
        if !self.claim_stream_crash(session) {
            return;
        }
        let state = app.state::<LiveSessionState>();
        let _ = super::actions::stop_live_runtime_after_crash(
            app.clone(),
            &state,
            self,
            session,
            message,
        );
    }

    fn claim_stream_crash(&self, session: u64) -> bool {
        session != 0
            && session & CRASH_CLAIM_BIT == 0
            && self
                .active_session
                .compare_exchange(
                    session,
                    session | CRASH_CLAIM_BIT,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
    }

    pub(crate) fn is_session_current(&self, session: u64) -> bool {
        active_session_matches(self.active_session.load(Ordering::SeqCst), session)
    }

    fn clear_active_session_if_current(&self, session: u64) -> bool {
        self.active_session
            .compare_exchange(session, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub(crate) fn claim_start_failure(&self, failure: LiveStartFailure) -> Option<String> {
        self.clear_active_session_if_current(failure.session)
            .then_some(failure.message)
    }
}

impl Default for LiveRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
fn stop_after_capture_for_test(
    adapter: &mut SessionAsrAdapter,
    finisher: &StreamFinisher,
    timeout: Duration,
) -> StreamFinishStatus {
    match adapter.drain_after_capture(timeout) {
        Ok(AdapterDrainStatus::Drained) => finisher.finish_session(),
        Ok(AdapterDrainStatus::TimedOut | AdapterDrainStatus::TimedOutRetained) => {
            StreamFinishStatus::TimedOut
        }
        Err(_) => StreamFinishStatus::Disconnected,
    }
}

fn spawn_stream_crash_handler(
    app: tauri::AppHandle,
    runtime: LiveRuntime,
    session: u64,
    message: String,
) {
    std::thread::spawn(move || runtime.handle_stream_crash(app, session, &message));
}

fn log_worker_shutdown_errors(errors: Vec<String>) {
    for error in errors {
        crate::stt::log_yap(&format!("live worker shutdown failed: {error}"));
    }
}

#[cfg(test)]
mod tests;
