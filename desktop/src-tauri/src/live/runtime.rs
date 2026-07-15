#[cfg(test)]
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
#[cfg(test)]
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::audio::coordinator::RECORDING_QUEUE_CAPACITY;
use crate::audio::recording::RecordingFinalizeResult;

use super::stream::LiveStreamEngine;
#[cfg(test)]
use super::stream::{self, StreamMessage};

mod asr_adapter;
mod capture_worker;
mod control;
mod finalization;
mod level_channel;
mod lifecycle_gate;
mod local_start;
mod resources;
mod session_control;
mod session_identity;
mod stop;
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
#[cfg(test)]
use session_identity::{active_session_matches, CRASH_CLAIM_BIT};
#[cfg(test)]
use stream_session::should_accept_stream_samples;
#[cfg(test)]
use stream_session::SessionStream;
pub use stream_session::StreamFinishStatus;
#[cfg(test)]
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
