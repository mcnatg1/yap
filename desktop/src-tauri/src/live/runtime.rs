#[cfg(test)]
use std::sync::mpsc;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tauri::Manager;

use crate::audio::capture::CaptureAdapter;
use crate::audio::coordinator::{bounded_sink, SinkKind, RECORDING_QUEUE_CAPACITY};
use crate::audio::recording::{RecordingFinalizeResult, RecordingSinkHandle};
use crate::audio::session::{SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode};

use super::state::{LiveLevelView, LiveSessionState};
use super::stream::LiveStreamEngine;
#[cfg(test)]
use super::stream::{self, StreamMessage};

mod asr_adapter;
mod capture_worker;
mod level_channel;
mod lifecycle_gate;
mod session_identity;
mod stream_session;
mod warmup;
mod worker;

#[cfg(test)]
use asr_adapter::set_reaper_spawn_failure_for_test;
use asr_adapter::{
    AdapterDrainStatus, PendingAsrAdapter, SessionAsrAdapter, ASR_ADAPTER_DRAIN_TIMEOUT,
};
#[cfg(test)]
use capture_worker::*;
use capture_worker::{run_capture_worker, CaptureWorkerContext};
#[cfg(test)]
use level_channel::publish_level;
use level_channel::{level_channel, LatestLevelReceiver};
use lifecycle_gate::{LifecycleGate, OwnedLifecycleOperation};
use session_identity::{active_session_matches, CRASH_CLAIM_BIT};
#[cfg(test)]
use stream_session::should_accept_stream_samples;
pub use stream_session::StreamFinishStatus;
use stream_session::{SessionStream, StreamFinisher};
use warmup::SharedWarmup;
use worker::join_worker;

const LEVEL_TICK: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct LiveRuntime {
    inner: Arc<Mutex<LiveRuntimeInner>>,
    active_session: Arc<AtomicU64>,
    start_generation: Arc<AtomicU64>,
    recording_finalization: Arc<RecordingFinalization>,
    stop_completion: Arc<StopCompletion>,
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

struct StopCompletion {
    state: Mutex<StopCompletionState>,
    completed: Condvar,
}

enum StopCompletionState {
    Pending,
    Finalizing,
    Finalized(Box<LiveStopResult>),
}

impl StopCompletion {
    fn new() -> Self {
        Self {
            state: Mutex::new(StopCompletionState::Pending),
            completed: Condvar::new(),
        }
    }

    fn reset(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "live stop completion state became unavailable")?;
        if matches!(*state, StopCompletionState::Finalizing) {
            return Err("Previous live stop is still finalizing.".into());
        }
        *state = StopCompletionState::Pending;
        Ok(())
    }
}

struct LiveRuntimeInner {
    session: u64,
    capture: Option<CaptureAdapter>,
    stream: Option<SessionStream>,
    retiring_stream: Option<SessionStream>,
    pending_asr: Option<PendingAsrAdapter>,
    asr_adapter: Option<SessionAsrAdapter>,
    recording: Option<RecordingSinkHandle>,
    level: Option<JoinHandle<()>>,
    last_used: Instant,
    #[cfg(test)]
    has_capture_for_test: bool,
    #[cfg(test)]
    has_stream_for_test: bool,
}

struct RecordingFinalization {
    state: Mutex<RecordingFinalizationState>,
    completed: Condvar,
}

enum RecordingFinalizationState {
    Pending,
    Finalizing,
    Finalized(Box<Option<RecordingFinalizeResult>>),
    Failed {
        error: String,
        session_id: Option<SessionId>,
    },
}

impl RecordingFinalization {
    fn new() -> Self {
        Self {
            state: Mutex::new(RecordingFinalizationState::Pending),
            completed: Condvar::new(),
        }
    }

    fn prepare_for_new_recording(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "recording finalization state became unavailable")?;
        if matches!(*state, RecordingFinalizationState::Finalizing) {
            return Err("Previous live recording is still finalizing.".into());
        }
        *state = RecordingFinalizationState::Pending;
        Ok(())
    }
}

struct RecordingFinalizationLease<'a> {
    finalization: &'a RecordingFinalization,
    completed: bool,
}

impl<'a> RecordingFinalizationLease<'a> {
    fn new(finalization: &'a RecordingFinalization) -> Self {
        Self {
            finalization,
            completed: false,
        }
    }

    fn finish(
        mut self,
        result: Result<Option<RecordingFinalizeResult>, String>,
        session_id: Option<SessionId>,
    ) -> Result<Option<RecordingFinalizeResult>, String> {
        let mut state = self
            .finalization
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = match &result {
            Ok(result) => RecordingFinalizationState::Finalized(Box::new(result.clone())),
            Err(error) => RecordingFinalizationState::Failed {
                error: error.clone(),
                session_id,
            },
        };
        self.completed = true;
        self.finalization.completed.notify_all();
        result
    }
}

impl Drop for RecordingFinalizationLease<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut state = self
            .finalization
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(*state, RecordingFinalizationState::Finalizing) {
            *state = RecordingFinalizationState::Failed {
                error: "recording finalization interrupted before completion".into(),
                session_id: None,
            };
        }
        self.finalization.completed.notify_all();
    }
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

pub(crate) struct LocalCaptureStart {
    session: u64,
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
            inner: Arc::new(Mutex::new(LiveRuntimeInner::new())),
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
            .capture
            .is_some()
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
        if inner.capture.is_some() {
            return Err("Stop live before changing local fallback.".to_string());
        }
        inner.retire_stream();
        drop(inner);
        self.model_warmup.clear_idle()?;
        Ok(lease)
    }

    pub(crate) fn start_local_capture(
        &self,
        app: tauri::AppHandle,
        selected_device_id: Option<String>,
        capture_mode: super::state::LiveCaptureMode,
        intent: StartIntent,
    ) -> Result<Option<LocalCaptureStart>, LiveStartFailure> {
        let session = {
            let inner = self.inner.lock().expect("live runtime poisoned");
            if inner.capture.is_some() {
                return Ok(None);
            }
            drop(inner);
            self.ensure_recording_ready_to_start()
                .map_err(|message| LiveStartFailure::new(0, message))?;
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if inner.capture.is_some() {
                return Ok(None);
            }
            inner.session = inner.session.saturating_add(1);
            inner.last_used = Instant::now();
            let session = inner.session;
            self.active_session.store(session, Ordering::SeqCst);
            session
        };

        if !self.start_intent_is_current(intent) {
            return Ok(None);
        }

        let resolved = match super::devices::resolve_capture_device(selected_device_id.as_deref()) {
            Ok(resolved) => resolved,
            Err(error) => return Err(LiveStartFailure::new(session, error)),
        };
        let stream_config = resolved.config.config();
        let sample_format = resolved.config.sample_format();
        let (level_tx, level) = level_channel();
        let pending_asr = PendingAsrAdapter::new();
        let local_asr = pending_asr.sink();
        let capture_runtime = self.clone();
        let capture_app = app.clone();
        let capture_active_session = Arc::clone(&self.active_session);
        let (recording_sink, recording_rx) =
            bounded_sink(SinkKind::Recording, RECORDING_QUEUE_CAPACITY);
        let recording_directory = super::recordings::recordings_dir();
        let recording_reservation =
            crate::audio::recording::allocate_recording_session(&recording_directory)
                .map_err(|message| LiveStartFailure::new(session, message))?;
        let recording_session_id = recording_reservation.session_id().clone();
        let trigger_mode = match capture_mode {
            super::state::LiveCaptureMode::PushToTalk => TriggerMode::PushToTalk,
            super::state::LiveCaptureMode::Toggle => TriggerMode::Toggle,
        };
        let session_metadata = SessionMetadata::new(
            recording_session_id.clone(),
            SessionMode::Dictation,
            SessionOrigin::LiveCapture,
            trigger_mode,
            std::time::SystemTime::now(),
            None,
            None,
            None,
            Vec::new(),
            None,
        )
        .map_err(|message| LiveStartFailure::new(session, message))?;
        let recording_reservation = recording_reservation
            .with_session_metadata(session_metadata)
            .map_err(|message| LiveStartFailure::new(session, message))?;
        let recording_handle = RecordingSinkHandle::spawn_reserved(
            recording_reservation,
            recording_sink,
            recording_rx,
        );
        let recording_for_capture = recording_handle.sink();
        if !self.start_intent_is_current(intent)
            || self.active_session.load(Ordering::Acquire) != session
        {
            let _ = recording_handle.abort("live start cancelled before capture opened");
            return Ok(None);
        }
        let capture = match CaptureAdapter::open(
            resolved.device,
            stream_config,
            sample_format,
            move |ports, errors| {
                run_capture_worker(
                    ports,
                    errors,
                    CaptureWorkerContext {
                        runtime: capture_runtime,
                        app: capture_app,
                        session,
                        recording_session_id,
                        active_session: capture_active_session,
                        recording: recording_for_capture,
                        local_asr,
                        level_tx,
                    },
                );
            },
        ) {
            Ok(capture) => capture,
            Err(error) => {
                if let Err(finalize_error) =
                    recording_handle.abort(format!("capture adapter failed to open: {error}"))
                {
                    crate::stt::log_yap(&format!(
                        "live recording abort after capture-open failure failed: {finalize_error}"
                    ));
                }
                return Err(LiveStartFailure::new(session, error));
            }
        };
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if !should_install_capture(
            session,
            inner.session,
            self.active_session.load(Ordering::SeqCst),
            inner.capture.is_some(),
        ) {
            inner.last_used = Instant::now();
            drop(inner);
            if let Err(error) = capture.shutdown() {
                crate::stt::log_yap(&format!("live capture shutdown failed: {error}"));
            }
            let _ = recording_handle.finalize();
            drop(level);
            return Ok(None);
        }
        inner.capture = Some(capture);
        inner.recording = Some(recording_handle);
        inner.pending_asr = Some(pending_asr);
        inner.start_level_worker(
            app.clone(),
            level,
            session,
            Arc::clone(&self.active_session),
        );
        drop(inner);

        let state = app.state::<LiveSessionState>();
        let Some(view) = state.try_begin_listening_from_armed() else {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let (shutdown_errors, _) = inner.stop_capture();
            drop(inner);
            log_worker_shutdown_errors(shutdown_errors);
            let _ = self.finalize_recording();
            return Ok(None);
        };
        super::events::emit_session(&app, &view);
        Ok(Some(LocalCaptureStart { session }))
    }

    pub(crate) fn complete_local_start(
        &self,
        app: tauri::AppHandle,
        start: LocalCaptureStart,
        intent: StartIntent,
    ) -> Result<bool, LiveStartFailure> {
        let session = start.session;
        let reused = self.run_start_lifecycle(intent, || {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if !self.capture_session_is_current(&inner, session) {
                return Ok(false);
            }
            if inner.reuse_stream(session)? {
                inner.start_pending_asr_adapter(session)?;
                return Ok(true);
            }
            Ok(false)
        });
        match reused {
            None => return Ok(false),
            Some(Ok(true)) => return Ok(true),
            Some(Ok(false)) => {}
            Some(Err(message)) => return Err(LiveStartFailure::new(session, message)),
        }

        self.request_model_warmup()
            .map_err(|message| LiveStartFailure::new(session, message))?;
        let Some(model) = self
            .model_warmup
            .wait_cancellable(|| !self.start_intent_is_current(intent))
            .map_err(|message| LiveStartFailure::new(session, message))?
        else {
            return Ok(false);
        };
        let model_warmup = Arc::clone(&self.model_warmup);
        self.run_start_lifecycle(intent, move || {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if !self.capture_session_is_current(&inner, session) {
                return Ok(false);
            }
            if !inner.reuse_stream(session)? {
                inner.install_stream(self.clone(), app, session, model.commit(), model_warmup)?;
            }
            inner.start_pending_asr_adapter(session)?;
            Ok(true)
        })
        .unwrap_or(Ok(false))
        .map_err(|message| LiveStartFailure::new(session, message))
    }

    fn capture_session_is_current(&self, inner: &LiveRuntimeInner, session: u64) -> bool {
        session != 0
            && inner.session == session
            && inner.capture.is_some()
            && self.active_session.load(Ordering::Acquire) == session
    }

    pub fn request_warm(&self, _app: tauri::AppHandle) -> Result<bool, String> {
        if self.model_mutation_active.load(Ordering::Acquire) {
            return Ok(false);
        }
        if self
            .inner
            .lock()
            .expect("live runtime poisoned")
            .stream
            .as_ref()
            .is_some_and(SessionStream::is_running)
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
        inner.last_used = Instant::now();
        finish_status
    }

    pub(crate) fn finish_stop(&self, stream: StreamFinishStatus) -> LiveStopResult {
        let should_finalize = {
            let mut state = self
                .stop_completion
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            loop {
                match &*state {
                    StopCompletionState::Finalized(result) => return (**result).clone(),
                    StopCompletionState::Pending => {
                        *state = StopCompletionState::Finalizing;
                        break true;
                    }
                    StopCompletionState::Finalizing => {
                        state = self
                            .stop_completion
                            .completed
                            .wait(state)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                }
            }
        };
        debug_assert!(should_finalize);
        let result = LiveStopResult {
            stream,
            recording: self.finalize_recording(),
        };
        let mut state = self
            .stop_completion
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = StopCompletionState::Finalized(Box::new(result.clone()));
        self.stop_completion.completed.notify_all();
        result
    }

    pub fn unload_if_idle(&self, threshold: Duration) {
        self.run_stop_lifecycle(|| {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if inner.capture.is_none() && inner.last_used.elapsed() >= threshold {
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
        let lease = {
            let mut state = self
                .recording_finalization
                .state
                .lock()
                .map_err(|_| "recording finalization state became unavailable")?;
            loop {
                match &*state {
                    RecordingFinalizationState::Finalized(result) => return Ok((**result).clone()),
                    RecordingFinalizationState::Failed { error, .. } => return Err(error.clone()),
                    RecordingFinalizationState::Pending => {
                        *state = RecordingFinalizationState::Finalizing;
                        break RecordingFinalizationLease::new(&self.recording_finalization);
                    }
                    RecordingFinalizationState::Finalizing => {
                        state = self
                            .recording_finalization
                            .completed
                            .wait(state)
                            .map_err(|_| "recording finalization state became unavailable")?;
                    }
                }
            }
        };
        let (result, session_id) = match self.inner.lock() {
            Ok(mut inner) => {
                let recording = inner.recording.take();
                let session_id = recording
                    .as_ref()
                    .map(|recording| recording.session_id().clone());
                (
                    recording.map(|recording| recording.finalize()).transpose(),
                    session_id,
                )
            }
            Err(_) => (Err("live runtime became unavailable".into()), None),
        };
        lease.finish(result, session_id)
    }

    pub(crate) fn recording_finalization_failure(&self) -> Option<(SessionId, String)> {
        let state = self.recording_finalization.state.lock().ok()?;
        match &*state {
            RecordingFinalizationState::Failed {
                error,
                session_id: Some(session_id),
            } => Some((session_id.clone(), error.clone())),
            _ => None,
        }
    }

    fn ensure_recording_ready_to_start(&self) -> Result<(), String> {
        let prior_recording = self
            .inner
            .lock()
            .map_err(|_| "live runtime became unavailable")?
            .recording
            .is_some();
        if prior_recording {
            return Err("Previous live recording must be finalized before starting again.".into());
        }
        self.recording_finalization.prepare_for_new_recording()?;
        self.stop_completion.reset()
    }

    #[cfg(test)]
    pub(crate) fn install_unavailable_recording_for_test(&self, session_id: SessionId) {
        let (sink, _receiver) = bounded_sink(SinkKind::Recording, 1);
        self.inner.lock().unwrap().recording = Some(
            RecordingSinkHandle::spawn_unavailable_for_test(sink, session_id),
        );
    }

    #[cfg(test)]
    pub(crate) fn install_panicking_recording_for_test(&self, session_id: SessionId) {
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        self.inner.lock().unwrap().recording = Some(RecordingSinkHandle::spawn_panicking_for_test(
            sink, receiver, session_id,
        ));
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

impl LiveRuntimeInner {
    fn new() -> Self {
        Self {
            session: 0,
            capture: None,
            stream: None,
            retiring_stream: None,
            pending_asr: None,
            asr_adapter: None,
            recording: None,
            level: None,
            last_used: Instant::now(),
            #[cfg(test)]
            has_capture_for_test: false,
            #[cfg(test)]
            has_stream_for_test: false,
        }
    }

    fn reuse_stream(&mut self, session: u64) -> Result<bool, String> {
        self.reap_retiring_stream()?;
        if let Some(stream) = self.stream.as_ref().filter(|stream| stream.is_running()) {
            stream.retarget(session);
            return Ok(true);
        }
        self.retire_stream();
        Ok(false)
    }

    fn install_stream(
        &mut self,
        runtime: LiveRuntime,
        app: tauri::AppHandle,
        session: u64,
        engine: LiveStreamEngine,
        model_warmup: Arc<SharedWarmup<LiveStreamEngine>>,
    ) -> Result<(), String> {
        self.stream = Some(SessionStream::start(
            engine,
            session,
            Arc::clone(&runtime.active_session),
            app,
            model_warmup,
        ));
        Ok(())
    }

    fn start_pending_asr_adapter(&mut self, session: u64) -> Result<(), String> {
        self.cancel_asr_adapter()?;
        let samples_tx = self
            .stream
            .as_ref()
            .map(SessionStream::sender)
            .ok_or_else(|| "Live stream is unavailable.".to_string())?;
        let pending = self
            .pending_asr
            .take()
            .ok_or_else(|| "Live pre-roll is unavailable.".to_string())?;
        let adapter = pending.start(samples_tx, session);
        self.asr_adapter = Some(adapter);
        Ok(())
    }

    fn start_level_worker(
        &mut self,
        app: tauri::AppHandle,
        level: LatestLevelReceiver,
        session: u64,
        active_session: Arc<AtomicU64>,
    ) {
        if let Some(handle) = self.level.take() {
            if let Err(error) = join_worker(handle) {
                crate::stt::log_yap(&format!("live level worker shutdown failed: {error}"));
            }
        }
        let handle = std::thread::spawn(move || {
            let state = app.state::<LiveSessionState>();
            while let Ok(value) = level.recv() {
                if !active_session_matches(active_session.load(Ordering::SeqCst), session) {
                    break;
                }
                let view = state.update_level(value);
                let level = LiveLevelView::from(&view);
                super::events::emit_level(&app, &level);
                std::thread::sleep(LEVEL_TICK);
            }
        });
        self.level = Some(handle);
    }

    fn stop_capture(&mut self) -> (Vec<String>, Option<StreamFinishStatus>) {
        let mut errors = Vec::new();
        let mut adapter_status = None;
        if let Some(capture) = self.capture.take() {
            if let Err(error) = capture.shutdown() {
                errors.push(error);
            }
        }
        self.pending_asr.take();
        if let Some(mut adapter) = self.asr_adapter.take() {
            match adapter.drain_after_capture(ASR_ADAPTER_DRAIN_TIMEOUT) {
                Ok(AdapterDrainStatus::Drained) => {}
                Ok(AdapterDrainStatus::TimedOut | AdapterDrainStatus::TimedOutRetained) => {
                    adapter_status = Some(StreamFinishStatus::TimedOut);
                    if let Some(error) = adapter.take_cleanup_error() {
                        errors.push(error);
                    }
                    if adapter.retains_cleanup_ownership() {
                        self.asr_adapter = Some(adapter);
                    }
                }
                Err(error) => {
                    errors.push(error);
                    adapter_status = Some(StreamFinishStatus::Disconnected);
                    if adapter.retains_cleanup_ownership() {
                        self.asr_adapter = Some(adapter);
                    }
                }
            }
        }
        if let Some(handle) = self.level.take() {
            if let Err(error) = join_worker(handle) {
                errors.push(error);
            }
        }
        #[cfg(test)]
        {
            self.has_capture_for_test = false;
        }
        (errors, adapter_status)
    }

    fn retire_stream(&mut self) {
        if let Err(error) = self.cancel_asr_adapter() {
            crate::stt::log_yap(&format!("live ASR adapter shutdown failed: {error}"));
        }
        if let Some(stream) = self.stream.take() {
            if let Err(error) = stream.shutdown(true) {
                crate::stt::log_yap(&format!("live stream worker shutdown failed: {error}"));
            }
        }
        let retiring_finished = self
            .retiring_stream
            .as_ref()
            .is_some_and(SessionStream::is_finished);
        if retiring_finished {
            let stream = self
                .retiring_stream
                .take()
                .expect("finished retiring stream was present");
            if let Err(error) = stream.shutdown(true) {
                crate::stt::log_yap(&format!("retiring live stream shutdown failed: {error}"));
            }
        }
        #[cfg(test)]
        {
            self.has_stream_for_test = false;
        }
    }

    fn retire_stream_detached_reader(&mut self) {
        if let Err(error) = self.cancel_asr_adapter() {
            crate::stt::log_yap(&format!("live ASR adapter shutdown failed: {error}"));
        }
        if let Some(stream) = self.stream.take() {
            stream.cancel_reader();
            if self.retiring_stream.is_none() {
                self.retiring_stream = Some(stream);
            } else if let Err(error) = stream.shutdown(true) {
                crate::stt::log_yap(&format!("extra retiring stream shutdown failed: {error}"));
            }
        }
        #[cfg(test)]
        {
            self.has_stream_for_test = false;
        }
    }

    fn stream_finisher(&self) -> Option<StreamFinisher> {
        self.stream.as_ref().map(SessionStream::finisher)
    }

    fn reap_retiring_stream(&mut self) -> Result<(), String> {
        let Some(stream) = self.retiring_stream.as_ref() else {
            return Ok(());
        };
        if !stream.is_finished() {
            return Err("Previous live transcription is still stopping.".into());
        }
        let stream = self
            .retiring_stream
            .take()
            .expect("retiring stream was present");
        stream.shutdown(true)
    }

    fn cancel_asr_adapter(&mut self) -> Result<(), String> {
        if let Some(mut adapter) = self.asr_adapter.take() {
            if let Err(error) = adapter.cancel_and_join() {
                self.asr_adapter = Some(adapter);
                return Err(error);
            }
        }
        Ok(())
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

fn should_install_capture(
    requested_session: u64,
    inner_session: u64,
    active_session: u64,
    has_capture: bool,
) -> bool {
    requested_session != 0
        && requested_session == inner_session
        && requested_session == active_session
        && !has_capture
}

#[cfg(test)]
impl LiveRuntimeInner {
    fn for_test() -> Self {
        Self::new()
    }

    fn mark_stream_crashed_for_test(&mut self) {
        self.has_capture_for_test = false;
        self.has_stream_for_test = false;
    }
}

#[cfg(test)]
mod tests;
