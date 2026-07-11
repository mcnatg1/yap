use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Condvar, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

use crate::audio::capture::{CaptureAdapter, CapturePacket, CapturePorts};
use crate::audio::coordinator::{
    bounded_sink, BoundedReceiver, BoundedSink, Coordinator, CoordinatorPorts, SinkKind,
    LOCAL_ASR_QUEUE_CAPACITY, RECORDING_QUEUE_CAPACITY,
};
use crate::audio::frame::PreparedFrame;
use crate::audio::recording::{RecordingFinalizeResult, RecordingSinkHandle};
use crate::audio::session::{
    SessionId, SessionMetadata, SessionMode, SessionOrigin, TrackId, TriggerMode,
};

use super::state::{LiveLevelView, LiveSessionState};
use super::stream::{self, LiveStreamEngine, StreamMessage};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const LEVEL_TICK: Duration = Duration::from_millis(50);
const STREAM_FINISH_ENQUEUE_TIMEOUT: Duration = Duration::from_millis(250);
const STREAM_DRAIN_ON_STOP: Duration = Duration::from_millis(6000);
const ASR_ADAPTER_CANCEL_GRACE: Duration = Duration::from_millis(100);
const CRASH_CLAIM_BIT: u64 = 1 << 63;

fn active_session_matches(active_session: u64, session: u64) -> bool {
    session != 0 && (active_session == session || active_session == session | CRASH_CLAIM_BIT)
}

#[derive(Clone)]
pub struct LiveRuntime {
    inner: Arc<Mutex<LiveRuntimeInner>>,
    active_session: Arc<AtomicU64>,
    recording_finalization: Arc<RecordingFinalization>,
    stop_completion: Arc<StopCompletion>,
    transition: Arc<LifecycleGate>,
    warming: Arc<AtomicBool>,
}

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

struct LifecycleGate {
    state: Mutex<LifecycleState>,
    changed: Condvar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LifecycleState {
    Idle,
    Starting,
    Stopping,
}

struct LifecycleOperation<'a> {
    gate: &'a LifecycleGate,
}

struct SessionStream {
    session: Arc<AtomicU64>,
    samples_tx: mpsc::SyncSender<StreamMessage>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

struct SessionAsrAdapter {
    frames_tx: BoundedSink<PreparedFrame>,
    cancelled: Arc<AtomicBool>,
    completed_rx: Option<mpsc::Receiver<()>>,
    worker: Option<JoinHandle<()>>,
    cleanup_error: Option<String>,
}

struct AdapterReapPayload {
    worker: JoinHandle<()>,
    completed_rx: mpsc::Receiver<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterDrainStatus {
    Drained,
    TimedOut,
    TimedOutRetained,
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_REAPER_SPAWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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

impl LifecycleGate {
    fn new() -> Self {
        Self {
            state: Mutex::new(LifecycleState::Idle),
            changed: Condvar::new(),
        }
    }

    fn begin_start(&self) -> LifecycleOperation<'_> {
        self.begin(LifecycleState::Starting)
    }

    #[cfg(test)]
    fn begin_start_with_wait_hook<F>(&self, on_wait: F) -> LifecycleOperation<'_>
    where
        F: FnOnce(),
    {
        self.begin_with_wait_hook(LifecycleState::Starting, Some(on_wait))
    }

    fn begin_stop(&self) -> LifecycleOperation<'_> {
        self.begin(LifecycleState::Stopping)
    }

    fn begin(&self, next: LifecycleState) -> LifecycleOperation<'_> {
        self.begin_with_wait_hook(next, None::<fn()>)
    }

    fn begin_with_wait_hook<F>(
        &self,
        next: LifecycleState,
        mut on_wait: Option<F>,
    ) -> LifecycleOperation<'_>
    where
        F: FnOnce(),
    {
        let mut state = self.state.lock().expect("live transition gate poisoned");
        while *state != LifecycleState::Idle {
            if let Some(on_wait) = on_wait.take() {
                on_wait();
            }
            state = self
                .changed
                .wait(state)
                .expect("live transition gate poisoned");
        }
        *state = next;
        LifecycleOperation { gate: self }
    }

    fn complete(&self) {
        let mut state = self.state.lock().expect("live transition gate poisoned");
        *state = LifecycleState::Idle;
        self.changed.notify_all();
    }
}

impl Drop for LifecycleOperation<'_> {
    fn drop(&mut self) {
        self.gate.complete();
    }
}

impl LiveRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LiveRuntimeInner::new())),
            active_session: Arc::new(AtomicU64::new(0)),
            recording_finalization: Arc::new(RecordingFinalization::new()),
            stop_completion: Arc::new(StopCompletion::new()),
            transition: Arc::new(LifecycleGate::new()),
            warming: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_active(&self) -> bool {
        self.inner
            .lock()
            .expect("live runtime poisoned")
            .capture
            .is_some()
    }

    pub(crate) fn start_local(
        &self,
        app: tauri::AppHandle,
        selected_device_id: Option<String>,
        capture_mode: super::state::LiveCaptureMode,
    ) -> Result<(), LiveStartFailure> {
        let _transition = self.transition.begin_start();
        let (session, local_asr) = {
            let inner = self.inner.lock().expect("live runtime poisoned");
            if inner.capture.is_some() {
                return Ok(());
            }
            drop(inner);
            self.ensure_recording_ready_to_start()
                .map_err(|message| LiveStartFailure::new(0, message))?;
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if inner.capture.is_some() {
                return Ok(());
            }
            inner.session = inner.session.saturating_add(1);
            inner.last_used = Instant::now();
            let session = inner.session;
            self.active_session.store(session, Ordering::SeqCst);
            if let Err(message) = inner.ensure_stream(self.clone(), app.clone(), session) {
                return Err(LiveStartFailure::new(session, message));
            }
            let local_asr = inner
                .start_asr_adapter(session)
                .map_err(|message| LiveStartFailure::new(session, message))?;
            (session, local_asr)
        };

        let state = app.state::<LiveSessionState>();
        let Some(view) = state.try_begin_listening_from_armed() else {
            let _ = self.active_session.compare_exchange(
                session,
                0,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            let _ = self
                .inner
                .lock()
                .expect("live runtime poisoned")
                .cancel_asr_adapter();
            return Ok(());
        };
        let _ = app.emit("live-session", &view);

        let resolved = match super::devices::resolve_capture_device(selected_device_id.as_deref()) {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ = self
                    .inner
                    .lock()
                    .expect("live runtime poisoned")
                    .cancel_asr_adapter();
                return Err(LiveStartFailure::new(session, error));
            }
        };
        let stream_config = resolved.config.config();
        let sample_format = resolved.config.sample_format();
        let (level_tx, level) = mpsc::channel::<f32>();
        let capture_runtime = self.clone();
        let capture_app = app.clone();
        let capture_active_session = Arc::clone(&self.active_session);
        let (recording_sink, recording_rx) =
            bounded_sink(SinkKind::Recording, RECORDING_QUEUE_CAPACITY);
        let recording_directory = super::recordings::recordings_dir();
        let recording_reservation =
            crate::audio::recording::allocate_recording_session(&recording_directory)
                .map_err(|message| LiveStartFailure::new(session, message))?;
        let trigger_mode = match capture_mode {
            super::state::LiveCaptureMode::PushToTalk => TriggerMode::PushToTalk,
            super::state::LiveCaptureMode::Toggle => TriggerMode::Toggle,
        };
        let session_metadata = SessionMetadata::new(
            recording_reservation.session_id().clone(),
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
                let _ = self
                    .inner
                    .lock()
                    .expect("live runtime poisoned")
                    .cancel_asr_adapter();
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
            let _ = self
                .inner
                .lock()
                .expect("live runtime poisoned")
                .cancel_asr_adapter();
            drop(level);
            return Ok(());
        }
        inner.capture = Some(capture);
        inner.recording = Some(recording_handle);
        inner.start_level_worker(app, level, session, Arc::clone(&self.active_session));
        Ok(())
    }

    pub fn warm(&self, app: tauri::AppHandle) -> Result<(), String> {
        let _transition = self.transition.begin_start();
        if !claim_warmup(&self.warming) {
            return Ok(());
        }
        let result = {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let session = inner.session;
            inner.ensure_stream(self.clone(), app, session)
        };
        release_warmup(&self.warming);
        result
    }

    pub fn stop(&self) -> LiveStopResult {
        let stream = self.stop_stream();
        self.finish_stop(stream)
    }

    pub(crate) fn stop_stream(&self) -> StreamFinishStatus {
        let _transition = self.transition.begin_stop();
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
        let _transition = self.transition.begin_stop();
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if inner.capture.is_none() && inner.last_used.elapsed() >= threshold {
            inner.retire_stream();
        }
    }

    pub fn shutdown(&self) {
        let _transition = self.transition.begin_stop();
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        let (shutdown_errors, _) = inner.stop_capture();
        inner.retire_stream();
        self.active_session.store(0, Ordering::SeqCst);
        drop(inner);
        let _ = self.finalize_recording();
        log_worker_shutdown_errors(shutdown_errors);
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
        let orchestrator = app.state::<crate::runtime::RuntimeOrchestratorState>();
        let _ = super::actions::stop_live_runtime_after_crash(
            app.clone(),
            &state,
            self,
            &orchestrator,
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

    fn ensure_stream(
        &mut self,
        runtime: LiveRuntime,
        app: tauri::AppHandle,
        session: u64,
    ) -> Result<(), String> {
        if self.stream.as_ref().is_some_and(SessionStream::is_running) {
            if let Some(stream) = self.stream.as_ref() {
                stream.session.store(session, Ordering::SeqCst);
            }
            return Ok(());
        }
        self.retire_stream();

        let engine = LiveStreamEngine::new().map_err(|err| err.user_message().to_string())?;
        let (samples_tx, samples_rx) = mpsc::sync_channel::<StreamMessage>(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream_session = Arc::new(AtomicU64::new(session));

        let worker = std::thread::spawn({
            let active_session = Arc::clone(&runtime.active_session);
            let stream_session = Arc::clone(&stream_session);
            let cancelled = Arc::clone(&cancelled);
            move || {
                run_stream_worker(
                    engine,
                    samples_rx,
                    stream_session,
                    active_session,
                    cancelled,
                    app,
                )
            }
        });

        self.stream = Some(SessionStream {
            session: stream_session,
            samples_tx,
            cancelled,
            worker: Some(worker),
        });
        Ok(())
    }

    fn start_asr_adapter(&mut self, session: u64) -> Result<BoundedSink<PreparedFrame>, String> {
        self.cancel_asr_adapter()?;
        let samples_tx = self
            .stream
            .as_ref()
            .map(|stream| stream.samples_tx.clone())
            .ok_or_else(|| "Live stream is unavailable.".to_string())?;
        let adapter = SessionAsrAdapter::start(samples_tx, session);
        let frames_tx = adapter.sink();
        self.asr_adapter = Some(adapter);
        Ok(frames_tx)
    }

    fn start_level_worker(
        &mut self,
        app: tauri::AppHandle,
        level: mpsc::Receiver<f32>,
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
            while let Ok(first) = level.recv() {
                let mut value = first;
                while let Ok(next) = level.try_recv() {
                    value = next;
                }
                if !active_session_matches(active_session.load(Ordering::SeqCst), session) {
                    break;
                }
                let view = state.update_level(value);
                let level = LiveLevelView::from(&view);
                let _ = app.emit("live-level", &level);
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
        if let Some(mut adapter) = self.asr_adapter.take() {
            match adapter.drain_after_capture(STREAM_DRAIN_ON_STOP) {
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
            if let Err(error) = stream.shutdown(false) {
                crate::stt::log_yap(&format!("live stream worker shutdown failed: {error}"));
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

impl SessionStream {
    fn is_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }

    fn finisher(&self) -> StreamFinisher {
        StreamFinisher {
            samples_tx: self.samples_tx.clone(),
            session: self.session.load(Ordering::SeqCst),
        }
    }

    fn shutdown(mut self, join_reader: bool) -> Result<(), String> {
        self.cancelled.store(true, Ordering::SeqCst);
        drop(self.samples_tx);
        if join_reader {
            if let Some(handle) = self.worker.take() {
                return join_worker(handle);
            }
        }
        Ok(())
    }
}

impl SessionAsrAdapter {
    fn start(samples_tx: mpsc::SyncSender<StreamMessage>, session: u64) -> Self {
        Self::start_with_completion_hook(samples_tx, session, || {})
    }

    fn start_with_completion_hook<F>(
        samples_tx: mpsc::SyncSender<StreamMessage>,
        session: u64,
        completion_hook: F,
    ) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let (frames_tx, frames_rx) = bounded_sink(SinkKind::LocalAsr, LOCAL_ASR_QUEUE_CAPACITY);
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let (completed_tx, completed_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            run_session_asr_adapter_worker(frames_rx, samples_tx, session, worker_cancelled);
            completion_hook();
            let _ = completed_tx.send(());
        });
        Self {
            frames_tx,
            cancelled,
            completed_rx: Some(completed_rx),
            worker: Some(worker),
            cleanup_error: None,
        }
    }

    #[cfg(test)]
    fn start_with_completion_gate_for_test(
        samples_tx: mpsc::SyncSender<StreamMessage>,
        session: u64,
        completion_gate: Arc<std::sync::Barrier>,
    ) -> Self {
        Self::start_with_completion_hook(samples_tx, session, move || {
            completion_gate.wait();
        })
    }

    fn sink(&self) -> BoundedSink<PreparedFrame> {
        self.frames_tx.clone()
    }

    #[cfg(test)]
    fn join_after_capture(&mut self) -> Result<(), String> {
        self.drain_after_capture(STREAM_DRAIN_ON_STOP)
            .and_then(|status| match status {
                AdapterDrainStatus::Drained => Ok(()),
                AdapterDrainStatus::TimedOut | AdapterDrainStatus::TimedOutRetained => {
                    Err("ASR adapter drain timed out.".to_string())
                }
            })
    }

    fn drain_after_capture(&mut self, timeout: Duration) -> Result<AdapterDrainStatus, String> {
        self.frames_tx.close();
        match self.wait_for_completion(timeout)? {
            AdapterDrainStatus::Drained => Ok(AdapterDrainStatus::Drained),
            AdapterDrainStatus::TimedOut => {
                self.cancelled.store(true, Ordering::SeqCst);
                match self.close_and_reap_after_cancel() {
                    Ok(()) => Ok(AdapterDrainStatus::TimedOut),
                    Err(error) => {
                        self.cleanup_error = Some(error);
                        Ok(AdapterDrainStatus::TimedOutRetained)
                    }
                }
            }
            AdapterDrainStatus::TimedOutRetained => {
                unreachable!("only drain_after_capture retains adapter ownership")
            }
        }
    }

    fn wait_for_completion(&mut self, timeout: Duration) -> Result<AdapterDrainStatus, String> {
        let completed_rx = self
            .completed_rx
            .as_ref()
            .ok_or_else(|| "ASR adapter completion was already consumed.".to_string())?;
        match completed_rx.recv_timeout(timeout) {
            Ok(()) => {
                self.completed_rx.take();
                if let Some(worker) = self.worker.take() {
                    join_worker(worker)?;
                }
                Ok(AdapterDrainStatus::Drained)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(AdapterDrainStatus::TimedOut),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(worker) = self.worker.take() {
                    join_worker(worker)?;
                }
                Err("ASR adapter stopped without reporting completion.".to_string())
            }
        }
    }

    fn close_and_reap_after_cancel(&mut self) -> Result<(), String> {
        self.frames_tx.close();
        match self.wait_for_completion(ASR_ADAPTER_CANCEL_GRACE) {
            Ok(AdapterDrainStatus::Drained) => Ok(()),
            Ok(AdapterDrainStatus::TimedOut) => self.hand_off_worker(),
            Ok(AdapterDrainStatus::TimedOutRetained) => {
                unreachable!("only drain_after_capture retains adapter ownership")
            }
            Err(error) => Err(error),
        }
    }

    fn hand_off_worker(&mut self) -> Result<(), String> {
        let (Some(worker), Some(completed_rx)) = (self.worker.take(), self.completed_rx.take())
        else {
            return Ok(());
        };
        let payload = Arc::new(Mutex::new(Some(AdapterReapPayload {
            worker,
            completed_rx,
        })));
        let reaper_payload = Arc::clone(&payload);
        match spawn_adapter_reaper(move || {
            let payload = reaper_payload
                .lock()
                .expect("ASR adapter reaper payload poisoned")
                .take()
                .expect("ASR adapter reaper owns one payload");
            let _ = payload.completed_rx.recv();
            if let Err(error) = join_worker(payload.worker) {
                crate::stt::log_yap(&format!("live ASR adapter reaper failed: {error}"));
            }
        }) {
            Ok(_) => Ok(()),
            Err(error) => {
                let payload = payload
                    .lock()
                    .map_err(|_| "ASR adapter reaper payload became unavailable.".to_string())?
                    .take()
                    .expect("failed reaper spawn leaves the adapter payload owned locally");
                self.worker = Some(payload.worker);
                self.completed_rx = Some(payload.completed_rx);
                Err(error)
            }
        }
    }

    fn cancel_and_join(&mut self) -> Result<(), String> {
        self.cancelled.store(true, Ordering::SeqCst);
        self.close_and_reap_after_cancel()
    }

    fn take_cleanup_error(&mut self) -> Option<String> {
        self.cleanup_error.take()
    }

    fn retains_cleanup_ownership(&self) -> bool {
        self.worker.is_some() || self.completed_rx.is_some()
    }

    #[cfg(test)]
    fn retains_cleanup_ownership_for_test(&self) -> bool {
        self.worker.is_some() && self.completed_rx.is_some()
    }
}

fn spawn_adapter_reaper<F>(run: F) -> Result<JoinHandle<()>, String>
where
    F: FnOnce() + Send + 'static,
{
    #[cfg(test)]
    if FAIL_NEXT_REAPER_SPAWN.with(|fail| fail.replace(false)) {
        return Err("ASR adapter reaper could not start (synthetic failure).".to_string());
    }
    thread::Builder::new()
        .name("live-asr-adapter-reaper".to_string())
        .spawn(run)
        .map_err(|error| format!("ASR adapter reaper could not start: {error}"))
}

#[cfg(test)]
fn set_reaper_spawn_failure_for_test() {
    FAIL_NEXT_REAPER_SPAWN.with(|fail| fail.set(true));
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

struct StreamFinisher {
    samples_tx: mpsc::SyncSender<StreamMessage>,
    session: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFinishStatus {
    Completed,
    BackedUp,
    Disconnected,
    NoStream,
    TimedOut,
}

impl StreamFinishStatus {
    fn should_retire_stream(self) -> bool {
        !matches!(
            self,
            StreamFinishStatus::Completed | StreamFinishStatus::NoStream
        )
    }

    pub(crate) fn should_report(self) -> bool {
        !matches!(
            self,
            StreamFinishStatus::Completed | StreamFinishStatus::NoStream
        )
    }
}

impl StreamFinisher {
    fn finish_session(&self) -> StreamFinishStatus {
        let (done_tx, done_rx) = mpsc::channel();
        let mut message = StreamMessage::Finish {
            session: self.session,
            done: done_tx,
        };
        let started = Instant::now();

        loop {
            match self.samples_tx.try_send(message) {
                Ok(()) => {
                    return match done_rx.recv_timeout(STREAM_DRAIN_ON_STOP) {
                        Ok(status) => status,
                        Err(mpsc::RecvTimeoutError::Timeout) => StreamFinishStatus::TimedOut,
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            StreamFinishStatus::Disconnected
                        }
                    };
                }
                Err(mpsc::TrySendError::Full(returned)) => {
                    if started.elapsed() >= STREAM_FINISH_ENQUEUE_TIMEOUT {
                        return StreamFinishStatus::BackedUp;
                    }
                    message = returned;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return StreamFinishStatus::Disconnected;
                }
            }
        }
    }
}

struct CaptureWorkerContext {
    runtime: LiveRuntime,
    app: tauri::AppHandle,
    session: u64,
    active_session: Arc<AtomicU64>,
    recording: BoundedSink<PreparedFrame>,
    local_asr: BoundedSink<PreparedFrame>,
    level_tx: mpsc::Sender<f32>,
}

fn run_capture_worker(
    ports: CapturePorts,
    errors: mpsc::Receiver<cpal::StreamError>,
    context: CaptureWorkerContext,
) {
    let CaptureWorkerContext {
        runtime,
        app,
        session,
        active_session,
        recording,
        local_asr,
        level_tx,
    } = context;
    let packet_runtime = runtime.clone();
    let packet_app = app.clone();
    let error_runtime = runtime.clone();
    let error_app = app.clone();
    let loss_runtime = runtime.clone();
    let loss_app = app.clone();
    let coordinator = Arc::new(Mutex::new(Coordinator::new(
        SessionId::new(format!("live-{session}")).expect("live session IDs are valid"),
        TrackId::new("live-microphone").expect("static live track ID is valid"),
        CoordinatorPorts {
            recording,
            local_asr: Some(local_asr),
            speaker_evidence: None,
            server_transport: None,
        },
    )));
    let transcription_degraded = Arc::new(AtomicBool::new(false));
    let packet_coordinator = Arc::clone(&coordinator);
    let packet_degraded = Arc::clone(&transcription_degraded);
    let loss_coordinator = Arc::clone(&coordinator);
    run_guarded_capture_packet_worker(
        || {
            run_capture_packet_loop(
                ports,
                errors,
                move |packet, losses| {
                    if !active_session_matches(active_session.load(Ordering::SeqCst), session) {
                        return false;
                    }
                    let mut coordinator = match packet_coordinator.lock() {
                        Ok(coordinator) => coordinator,
                        Err(_) => return true,
                    };
                    match coordinator.consume(packet, losses) {
                        Ok(level) => {
                            if coordinator
                                .outcome(SinkKind::LocalAsr)
                                .is_some_and(|outcome| outcome.dropped_frames > 0)
                                && mark_local_asr_degraded_once(&packet_degraded)
                            {
                                let state = packet_app.state::<LiveSessionState>();
                                let view = state.mark_transcription_backpressure();
                                let _ = packet_app.emit("live-session", &view);
                            }
                            let _ = level_tx.send(level);
                            false
                        }
                        Err(message) => {
                            spawn_stream_crash_handler(
                                packet_app.clone(),
                                packet_runtime.clone(),
                                session,
                                message,
                            );
                            true
                        }
                    }
                },
                move |error| {
                    let message = format!("Microphone input stopped: {error}");
                    crate::stt::log_yap(&format!("live input stream error: {error}"));
                    spawn_stream_crash_handler(
                        error_app.clone(),
                        error_runtime.clone(),
                        session,
                        message,
                    );
                    true
                },
                move |loss| match loss {
                    Ok(snapshot) => match loss_coordinator.lock() {
                        Ok(mut coordinator) => coordinator.consume_loss(snapshot).is_err(),
                        Err(_) => true,
                    },
                    Err(_) => {
                        spawn_stream_crash_handler(
                            loss_app.clone(),
                            loss_runtime.clone(),
                            session,
                            "Microphone capture timing became invalid.".to_string(),
                        );
                        true
                    }
                },
            );
        },
        move |message| spawn_stream_crash_handler(app, runtime, session, message),
    );
    if let Ok(mut coordinator) = coordinator.lock() {
        for outcome in coordinator.outcomes() {
            crate::stt::log_yap(&format!(
                "audio sink {:?} accepted={} dropped={} closed={} error={:?}",
                outcome.kind,
                outcome.accepted_frames,
                outcome.dropped_frames,
                outcome.closed,
                outcome.error
            ));
        }
        coordinator.close();
    };
}

fn run_guarded_capture_packet_worker<R, C>(run: R, process_crash: C)
where
    R: FnOnce(),
    C: FnOnce(String),
{
    if catch_unwind(AssertUnwindSafe(run)).is_err() {
        process_crash("Live capture worker stopped unexpectedly.".to_string());
    }
}

fn run_capture_packet_loop<P, E, L>(
    ports: CapturePorts,
    errors: mpsc::Receiver<cpal::StreamError>,
    process_packet: P,
    process_error: E,
    process_loss: L,
) where
    P: FnMut(&CapturePacket, &crate::audio::timeline::LossAccumulator) -> bool,
    E: FnMut(cpal::StreamError) -> bool,
    L: FnMut(
        Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    ) -> bool,
{
    run_capture_packet_loop_with_timeout(
        ports,
        errors,
        Duration::from_millis(50),
        process_packet,
        process_error,
        process_loss,
    );
}

fn run_capture_packet_loop_with_timeout<P, E, L>(
    ports: CapturePorts,
    errors: mpsc::Receiver<cpal::StreamError>,
    receive_timeout: Duration,
    mut process_packet: P,
    mut process_error: E,
    mut process_loss: L,
) where
    P: FnMut(&CapturePacket, &crate::audio::timeline::LossAccumulator) -> bool,
    E: FnMut(cpal::StreamError) -> bool,
    L: FnMut(
        Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    ) -> bool,
{
    let CapturePorts {
        packets,
        returned_buffers,
        losses,
    } = ports;
    loop {
        if drain_capture_losses(&losses, &mut process_loss) {
            break;
        }
        let mut should_exit = false;
        while let Ok(error) = errors.try_recv() {
            if process_error(error) {
                should_exit = true;
                break;
            }
        }
        if should_exit {
            break;
        }
        match packets.recv_timeout(receive_timeout) {
            Ok(mut packet) => {
                let should_exit = process_packet(&packet, &losses);
                packet.samples.clear();
                let _ = returned_buffers.try_send(packet.samples);
                if should_exit {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = drain_capture_losses(&losses, &mut process_loss);
}

fn drain_capture_losses<L>(
    losses: &crate::audio::timeline::LossAccumulator,
    process_loss: &mut L,
) -> bool
where
    L: FnMut(
        Result<crate::audio::timeline::LossSnapshot, crate::audio::timeline::TimelineError>,
    ) -> bool,
{
    match losses.try_drain() {
        Ok(crate::audio::timeline::TryDrain::Snapshot(snapshot)) => process_loss(Ok(snapshot)),
        Ok(crate::audio::timeline::TryDrain::Pending | crate::audio::timeline::TryDrain::Empty) => {
            false
        }
        Err(error) => process_loss(Err(error)),
    }
}

fn mark_local_asr_degraded_once(reported: &AtomicBool) -> bool {
    reported
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn spawn_stream_crash_handler(
    app: tauri::AppHandle,
    runtime: LiveRuntime,
    session: u64,
    message: String,
) {
    std::thread::spawn(move || runtime.handle_stream_crash(app, session, &message));
}

fn join_worker(handle: JoinHandle<()>) -> Result<(), String> {
    if handle.thread().id() == thread::current().id() {
        return Err("Worker attempted to join itself.".to_string());
    }
    handle
        .join()
        .map_err(|_| "Worker panicked during shutdown.".to_string())
}

fn log_worker_shutdown_errors(errors: Vec<String>) {
    for error in errors {
        crate::stt::log_yap(&format!("live worker shutdown failed: {error}"));
    }
}

fn claim_warmup(warming: &AtomicBool) -> bool {
    warming
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

fn release_warmup(warming: &AtomicBool) {
    warming.store(false, Ordering::Release);
}

fn run_stream_worker(
    mut engine: LiveStreamEngine,
    samples_rx: mpsc::Receiver<StreamMessage>,
    stream_session: Arc<AtomicU64>,
    active_session: Arc<AtomicU64>,
    cancelled: Arc<AtomicBool>,
    app: tauri::AppHandle,
) {
    let mut active_stream_session = 0;
    let mut buffer = Vec::<f32>::with_capacity(stream::chunk_samples() * 2);
    let mut profile = StreamProfile::default();

    while !cancelled.load(Ordering::Relaxed) {
        match samples_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => process_stream_message(
                &mut engine,
                &mut buffer,
                &mut profile,
                &app,
                &active_session,
                &stream_session,
                &mut active_stream_session,
                message,
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn run_session_asr_adapter_worker(
    frames_rx: BoundedReceiver<PreparedFrame>,
    samples_tx: mpsc::SyncSender<StreamMessage>,
    session: u64,
    cancelled: Arc<AtomicBool>,
) {
    while !cancelled.load(Ordering::Acquire) {
        let frame = match frames_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let mut message = StreamMessage::from_prepared(session, frame);
        loop {
            if cancelled.load(Ordering::Acquire) {
                return;
            }
            match samples_tx.try_send(message) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(returned)) => {
                    message = returned;
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => return,
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_stream_message(
    engine: &mut LiveStreamEngine,
    buffer: &mut Vec<f32>,
    profile: &mut StreamProfile,
    app: &tauri::AppHandle,
    active_session: &Arc<AtomicU64>,
    stream_session: &Arc<AtomicU64>,
    active_stream_session: &mut u64,
    message: StreamMessage,
) {
    match message {
        StreamMessage::Samples { session, samples } => {
            if !should_accept_stream_samples(
                session,
                active_session.load(Ordering::SeqCst),
                stream_session.load(Ordering::SeqCst),
            ) {
                return;
            }
            if *active_stream_session != session {
                engine.reset();
                buffer.clear();
                *profile = StreamProfile::new(session);
                *active_stream_session = session;
            }
            buffer.extend(samples);
            drain_stream_buffer(engine, buffer, profile, app, false);
        }
        StreamMessage::Finish { session, done } => {
            if *active_stream_session == session {
                drain_stream_buffer(engine, buffer, profile, app, true);
                let started = Instant::now();
                let final_text = engine.finish();
                profile.decode_elapsed += started.elapsed();
                if let Some(text) = final_text {
                    emit_stream_final(app, session, &text);
                }
                crate::stt::log_stt(&profile.summary());
                engine.reset();
                buffer.clear();
                *active_stream_session = 0;
                let _ = done.send(StreamFinishStatus::Completed);
            } else {
                let _ = done.send(StreamFinishStatus::NoStream);
            }
        }
    }
}

fn drain_stream_buffer(
    engine: &mut LiveStreamEngine,
    buffer: &mut Vec<f32>,
    profile: &mut StreamProfile,
    app: &tauri::AppHandle,
    flush_all: bool,
) {
    let chunk = stream::chunk_samples();
    while buffer.len() >= chunk || (flush_all && !buffer.is_empty()) {
        let take = if buffer.len() >= chunk {
            chunk
        } else {
            buffer.len()
        };
        let samples = buffer.drain(..take).collect::<Vec<_>>();
        profile.audio_samples += samples.len();
        profile.chunks += 1;
        let started = Instant::now();
        let text = engine.accept_samples(&samples);
        profile.decode_elapsed += started.elapsed();
        if let Some(text) = text {
            profile.mark_first_text();
            emit_stream_partial(app, profile.session, &text);
        }
    }
}

fn emit_stream_partial(app: &tauri::AppHandle, session: u64, text: &str) {
    if !active_session_matches(
        app.state::<LiveRuntime>()
            .active_session
            .load(Ordering::SeqCst),
        session,
    ) {
        return;
    }
    let state = app.state::<LiveSessionState>();
    let view = state.update_partial(text);
    let _ = app.emit("live-session", &view);
}

fn emit_stream_final(app: &tauri::AppHandle, session: u64, text: &str) {
    if !active_session_matches(
        app.state::<LiveRuntime>()
            .active_session
            .load(Ordering::SeqCst),
        session,
    ) {
        return;
    }
    let state = app.state::<LiveSessionState>();
    let view = state.update_final(text);
    let _ = app.emit("live-session", &view);
    std::thread::sleep(Duration::from_millis(180));
    let view = state.return_to_listening();
    let _ = app.emit("live-session", &view);
}

#[derive(Default)]
struct StreamProfile {
    session: u64,
    started: Option<Instant>,
    first_text: Option<Duration>,
    decode_elapsed: Duration,
    audio_samples: usize,
    chunks: usize,
}

impl StreamProfile {
    fn new(session: u64) -> Self {
        Self {
            session,
            started: Some(Instant::now()),
            ..Default::default()
        }
    }

    fn mark_first_text(&mut self) {
        if self.first_text.is_none() {
            self.first_text = self.started.map(|started| started.elapsed());
        }
    }

    fn summary(&self) -> String {
        let audio_ms = self.audio_samples as u64 * 1000 / TARGET_SAMPLE_RATE as u64;
        let first_text_ms = self
            .first_text
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|| "none".into());
        format!(
            "live nemotron profile session={} chunks={} audio_ms={} decode_ms={} first_text_ms={}",
            self.session,
            self.chunks,
            audio_ms,
            self.decode_elapsed.as_millis(),
            first_text_ms
        )
    }
}

fn should_accept_stream_samples(
    message_session: u64,
    active_session: u64,
    stream_session: u64,
) -> bool {
    active_session_matches(active_session, message_session) && message_session == stream_session
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
mod tests {
    use super::*;
    use crate::audio::capture::{new_callback_boundary, CapturePacket, CapturePorts};
    use crate::audio::frame::{AudioFrame, PreparedFrame};
    use crate::audio::session::{SessionId, TrackId};
    use crate::audio::timeline::LossAccumulator;
    use std::sync::Barrier;

    #[test]
    fn stream_crash_retires_runtime_handles() {
        let mut inner = LiveRuntimeInner::for_test();
        inner.has_capture_for_test = true;
        inner.has_stream_for_test = true;

        inner.mark_stream_crashed_for_test();

        assert!(!inner.has_capture_for_test);
        assert!(!inner.has_stream_for_test);
    }

    #[test]
    fn capture_packet_worker_returns_buffer_and_joins_after_disconnect() {
        let (packet_tx, packet_rx) = mpsc::sync_channel(1);
        let (returned_tx, returned_rx) = mpsc::sync_channel(8);
        let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
        let ports = CapturePorts {
            packets: packet_rx,
            returned_buffers: returned_tx,
            losses: Arc::new(LossAccumulator::new()),
        };
        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_capture_packet_loop(ports, error_rx, |_, _| false, |_| false, |_| false);
            done_tx.send(()).unwrap();
        });
        let mut samples = Vec::with_capacity(4);
        samples.extend([0.25, -0.25]);
        let allocation = samples.as_ptr();
        packet_tx
            .send(CapturePacket {
                source_position_frames: 0,
                channels: 2,
                sample_rate_hz: 48_000,
                samples,
            })
            .unwrap();

        let returned = returned_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(returned.as_ptr(), allocation);
        assert!(returned.is_empty());
        drop(packet_tx);
        drop(error_tx);

        done_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn capture_packet_loop_drains_loss_on_timeout_without_packets() {
        let (packet_tx, packet_rx) = mpsc::sync_channel(1);
        let (returned_tx, _) = mpsc::sync_channel(8);
        let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
        let losses = Arc::new(LossAccumulator::new());
        let ports = CapturePorts {
            packets: packet_rx,
            returned_buffers: returned_tx,
            losses: Arc::clone(&losses),
        };
        let (loss_tx, loss_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_capture_packet_loop_with_timeout(
                ports,
                error_rx,
                Duration::from_millis(1),
                |_, _| false,
                |_| false,
                move |loss| loss_tx.send(loss).is_err(),
            );
        });

        std::thread::sleep(Duration::from_millis(5));
        losses.record(240, 160, crate::audio::frame::GapCause::SinkUnavailable);
        let snapshot = loss_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.first_source_position_frames, 240);
        assert_eq!(snapshot.dropped_frames, 160);
        assert_eq!(
            snapshot.cause,
            crate::audio::frame::GapCause::SinkUnavailable
        );

        drop(packet_tx);
        drop(error_tx);
        worker.join().unwrap();
    }

    #[test]
    fn capture_packet_loop_disconnects_while_a_loss_drain_is_pending() {
        let losses = Arc::new(LossAccumulator::new());
        let registration_started = Arc::new(Barrier::new(2));
        let release_registration = Arc::new(Barrier::new(2));
        let callback = {
            let losses = Arc::clone(&losses);
            let registration_started = Arc::clone(&registration_started);
            let release_registration = Arc::clone(&release_registration);
            std::thread::spawn(move || {
                losses.record_with_registration_hooks(
                    0,
                    1,
                    crate::audio::frame::GapCause::SinkUnavailable,
                    || {
                        registration_started.wait();
                        release_registration.wait();
                    },
                    || {},
                );
            })
        };
        registration_started.wait();

        let (packet_tx, packet_rx) = mpsc::sync_channel(1);
        let (returned_tx, _) = mpsc::sync_channel(8);
        let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
        let ports = CapturePorts {
            packets: packet_rx,
            returned_buffers: returned_tx,
            losses,
        };
        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_capture_packet_loop_with_timeout(
                ports,
                error_rx,
                Duration::from_secs(1),
                |_, _| false,
                |_| false,
                |_| false,
            );
            done_tx.send(()).unwrap();
        });

        drop(packet_tx);
        drop(error_tx);
        let exited = done_rx.recv_timeout(Duration::from_secs(1));

        release_registration.wait();
        callback.join().unwrap();
        worker.join().unwrap();
        assert!(exited.is_ok());
    }

    #[test]
    fn capture_packet_loop_periodically_drains_sustained_losses_with_honest_positions() {
        let (mut callback, ports) = new_callback_boundary(2, 48_000, 2, 0, 1_000).unwrap();
        let (error_tx, error_rx) = mpsc::sync_channel::<cpal::StreamError>(1);
        let (loss_tx, loss_rx) = mpsc::channel();
        let (packet_started_tx, packet_started_rx) = mpsc::channel();
        let (release_packet_tx, release_packet_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_capture_packet_loop_with_timeout(
                ports,
                error_rx,
                Duration::from_millis(1),
                move |_, _| {
                    packet_started_tx.send(()).unwrap();
                    release_packet_rx.recv().unwrap();
                    false
                },
                |_| false,
                move |loss| loss_tx.send(loss).is_err(),
            );
        });

        let mut next_source_position = 1_000_u64;
        for _ in 0..64 {
            loop {
                callback.write_f32_for_test(&[0.0_f32, 0.0]);
                next_source_position += 1;
                if packet_started_rx
                    .recv_timeout(Duration::from_millis(10))
                    .is_ok()
                {
                    break;
                }
                let snapshot = loss_rx
                    .recv_timeout(Duration::from_secs(1))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    snapshot.first_source_position_frames,
                    next_source_position - 1
                );
                assert_eq!(snapshot.dropped_frames, 1);
            }

            let first_lost_position = next_source_position;
            for _ in 0..8 {
                callback.write_f32_for_test(&[0.0_f32, 0.0]);
                next_source_position += 1;
            }
            release_packet_tx.send(()).unwrap();
            let snapshot = loss_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap();
            assert_eq!(snapshot.first_source_position_frames, first_lost_position);
            assert_eq!(snapshot.dropped_frames, 8);
            assert_eq!(
                snapshot.cause,
                crate::audio::frame::GapCause::SinkUnavailable
            );
        }

        drop(callback);
        drop(error_tx);
        worker.join().unwrap();
    }

    #[test]
    fn guarded_capture_packet_worker_reports_a_synthetic_panic_and_exits() {
        let (crash_tx, crash_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run_guarded_capture_packet_worker(
                || panic!("synthetic packet worker panic"),
                move |message| crash_tx.send(message).unwrap(),
            );
        });

        assert_eq!(
            crash_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "Live capture worker stopped unexpectedly."
        );
        worker.join().unwrap();
    }

    #[test]
    fn stale_pcm_is_discarded_after_session_changes() {
        assert!(should_accept_stream_samples(2, 2, 2));
        assert!(should_accept_stream_samples(2, 2 | CRASH_CLAIM_BIT, 2));
        assert!(!should_accept_stream_samples(1, 2, 2));
        assert!(!should_accept_stream_samples(2, 0, 2));
        assert!(!should_accept_stream_samples(2, 2, 0));
    }

    #[test]
    fn stale_capture_install_is_rejected_after_stop_or_new_session() {
        assert!(should_install_capture(2, 2, 2, false));
        assert!(!should_install_capture(2, 2, 0, false));
        assert!(!should_install_capture(2, 3, 2, false));
        assert!(!should_install_capture(2, 2, 2, true));
    }

    #[test]
    fn stale_stream_crash_cannot_claim_a_newer_session() {
        let runtime = LiveRuntime::new();
        runtime.active_session.store(7, Ordering::SeqCst);

        assert!(!runtime.claim_stream_crash(6));
        assert_eq!(runtime.active_session.load(Ordering::SeqCst), 7);
        assert!(runtime.claim_stream_crash(7));
        assert_eq!(
            runtime.active_session.load(Ordering::SeqCst),
            7 | CRASH_CLAIM_BIT
        );
        assert!(active_session_matches(
            runtime.active_session.load(Ordering::SeqCst),
            7
        ));
        assert!(runtime.is_session_current(7));
        assert!(!runtime.is_session_current(8));
        assert!(!runtime.claim_stream_crash(7));
        assert!(!runtime.claim_stream_crash(0));
    }

    #[test]
    fn stale_start_failure_cannot_clear_a_newer_session() {
        let runtime = LiveRuntime::new();
        runtime.active_session.store(8, Ordering::SeqCst);

        assert_eq!(
            runtime.claim_start_failure(LiveStartFailure::new(7, "old failure".into())),
            None
        );
        assert_eq!(runtime.active_session.load(Ordering::SeqCst), 8);
        assert_eq!(
            runtime.claim_start_failure(LiveStartFailure::new(8, "current failure".into())),
            Some("current failure".into())
        );
        assert_eq!(runtime.active_session.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn warmup_latch_allows_one_in_flight_warm() {
        let warming = AtomicBool::new(false);

        assert!(claim_warmup(&warming));
        assert!(!claim_warmup(&warming));
        release_warmup(&warming);
        assert!(claim_warmup(&warming));
    }

    #[test]
    fn concurrent_recording_finalizers_share_one_cached_result_and_one_worker_finalization() {
        let runtime = LiveRuntime::new();
        let directory =
            std::env::temp_dir().join(format!("yap-runtime-finalize-race-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        let session_id = SessionId::new("runtime-finalize-race").unwrap();
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        let (recording, finalization_count) =
            RecordingSinkHandle::spawn_with_finalization_counter_for_test(
                directory.clone(),
                session_id.clone(),
                sink,
                receiver,
            );
        runtime.inner.lock().unwrap().recording = Some(recording);
        let barrier = Arc::new(Barrier::new(3));
        let left_runtime = runtime.clone();
        let left_barrier = Arc::clone(&barrier);
        let left = std::thread::spawn(move || {
            left_barrier.wait();
            left_runtime.finalize_recording().unwrap()
        });
        let right_runtime = runtime.clone();
        let right_barrier = Arc::clone(&barrier);
        let right = std::thread::spawn(move || {
            right_barrier.wait();
            right_runtime.finalize_recording().unwrap()
        });

        barrier.wait();
        let left = left.join().unwrap();
        let right = right.join().unwrap();

        assert_eq!(left, right);
        assert_eq!(
            finalization_count.load(Ordering::SeqCst),
            1,
            "only one caller may close, join, and publish the recording"
        );
        assert!(directory
            .join(format!("live-{session_id}.commit.json"))
            .is_file());
        assert_eq!(runtime.finalize_recording().unwrap(), left);
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn racing_stops_share_one_live_stop_result_and_one_recording_finalization() {
        let runtime = LiveRuntime::new();
        let directory =
            std::env::temp_dir().join(format!("yap-runtime-stop-race-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        let session_id = SessionId::new("runtime-stop-race").unwrap();
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        let (recording, finalization_count) =
            RecordingSinkHandle::spawn_with_finalization_counter_for_test(
                directory.clone(),
                session_id,
                sink,
                receiver,
            );
        runtime.inner.lock().unwrap().recording = Some(recording);
        let barrier = Arc::new(Barrier::new(3));
        let left_runtime = runtime.clone();
        let left_barrier = Arc::clone(&barrier);
        let left = std::thread::spawn(move || {
            left_barrier.wait();
            left_runtime.stop()
        });
        let right_runtime = runtime.clone();
        let right_barrier = Arc::clone(&barrier);
        let right = std::thread::spawn(move || {
            right_barrier.wait();
            right_runtime.stop()
        });

        barrier.wait();
        let left = left.join().unwrap();
        let right = right.join().unwrap();

        assert_eq!(left, right);
        assert_eq!(
            finalization_count.load(Ordering::SeqCst),
            1,
            "racing stops must share the finalization lease"
        );
        assert_eq!(runtime.stop(), left);
        std::fs::remove_dir_all(directory).ok();
    }

    #[test]
    fn poisoned_runtime_inner_publishes_one_terminal_error_and_wakes_waiters() {
        let runtime = LiveRuntime::new();
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let poison_runtime = runtime.clone();
        let poisoner = std::thread::spawn(move || {
            let _inner = poison_runtime.inner.lock().unwrap();
            locked_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            panic!("injected live runtime poison");
        });
        locked_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let first_runtime = runtime.clone();
        let first = std::thread::spawn(move || first_runtime.finalize_recording());
        wait_for_recording_finalizing(&runtime);
        let second_runtime = runtime.clone();
        let second = std::thread::spawn(move || second_runtime.finalize_recording());

        release_tx.send(()).unwrap();
        assert!(poisoner.join().is_err());
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        let repeated = runtime.finalize_recording();

        assert_eq!(first, second);
        assert_eq!(first, repeated);
        assert_eq!(first.unwrap_err(), "live runtime became unavailable");
    }

    #[test]
    fn direct_stop_then_start_rejects_unconsumed_recording_until_finalized() {
        let runtime = LiveRuntime::new();
        let session_id = SessionId::new("s-direct-restart").unwrap();
        runtime.install_unavailable_recording_for_test(session_id.clone());

        assert_eq!(
            runtime.ensure_recording_ready_to_start(),
            Err("Previous live recording must be finalized before starting again.".into())
        );
        assert_eq!(
            runtime.finalize_recording(),
            Err("recording worker is unavailable".into())
        );
        assert_eq!(
            runtime.recording_finalization_failure(),
            Some((session_id, "recording worker is unavailable".into()))
        );
        assert!(runtime.ensure_recording_ready_to_start().is_ok());
    }

    #[test]
    fn direct_stop_then_successful_finalize_allows_the_next_start() {
        let runtime = LiveRuntime::new();
        let directory =
            std::env::temp_dir().join(format!("yap-runtime-direct-restart-{}", std::process::id()));
        std::fs::remove_dir_all(&directory).ok();
        let session_id = SessionId::new("s-direct-restart-success").unwrap();
        let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
        runtime.inner.lock().unwrap().recording = Some(RecordingSinkHandle::spawn(
            directory.clone(),
            session_id,
            sink,
            receiver,
        ));

        assert!(runtime.ensure_recording_ready_to_start().is_err());
        assert_eq!(
            runtime.finalize_recording().unwrap().unwrap().status,
            crate::audio::recording::CaptureStatus::Complete
        );
        assert!(runtime.ensure_recording_ready_to_start().is_ok());
        std::fs::remove_dir_all(directory).ok();
    }

    fn wait_for_recording_finalizing(runtime: &LiveRuntime) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if matches!(
                *runtime.recording_finalization.state.lock().unwrap(),
                RecordingFinalizationState::Finalizing
            ) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "recording finalization was not claimed"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn local_asr_degradation_is_marked_once_without_stopping_recording() {
        let degradation_reported = AtomicBool::new(false);

        assert!(mark_local_asr_degraded_once(&degradation_reported));
        assert!(!mark_local_asr_degraded_once(&degradation_reported));
        assert!(degradation_reported.load(Ordering::SeqCst));
    }

    #[test]
    fn asr_adapter_forwards_the_last_accepted_frame_before_it_joins() {
        let (samples_tx, samples_rx) = mpsc::sync_channel(1);
        let mut adapter = SessionAsrAdapter::start(samples_tx, 7);
        let port = adapter.sink();
        port.try_send(prepared_frame(0.25)).unwrap();
        port.close();

        adapter.join_after_capture().unwrap();
        match samples_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
            StreamMessage::Samples { session, samples } => {
                assert_eq!(session, 7);
                assert_eq!(samples, vec![0.25]);
            }
            StreamMessage::Finish { .. } => panic!("expected the accepted frame"),
        }
    }

    #[test]
    fn stalled_recognizer_times_out_stop_without_enqueuing_finish() {
        let (samples_tx, samples_rx) = mpsc::sync_channel(1);
        samples_tx
            .try_send(StreamMessage::Samples {
                session: 7,
                samples: vec![0.0],
            })
            .unwrap();
        let mut adapter = SessionAsrAdapter::start(samples_tx.clone(), 7);
        let port = adapter.sink();
        port.try_send(prepared_frame(0.25)).unwrap();
        port.close();
        let finisher = StreamFinisher {
            samples_tx,
            session: 7,
        };

        let started = Instant::now();
        let status =
            stop_after_capture_for_test(&mut adapter, &finisher, Duration::from_millis(25));

        assert_eq!(status, StreamFinishStatus::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(adapter.worker.is_none());
        assert!(matches!(
            samples_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            StreamMessage::Samples { .. }
        ));
        assert!(matches!(
            samples_rx.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
    }

    #[test]
    fn reaper_spawn_failure_retains_adapter_ownership_and_reports_a_bounded_stop() {
        let (samples_tx, samples_rx) = mpsc::sync_channel(1);
        samples_tx
            .try_send(StreamMessage::Samples {
                session: 7,
                samples: vec![0.0],
            })
            .unwrap();
        let completion_gate = Arc::new(Barrier::new(2));
        let mut adapter = SessionAsrAdapter::start_with_completion_gate_for_test(
            samples_tx.clone(),
            7,
            Arc::clone(&completion_gate),
        );
        let port = adapter.sink();
        port.try_send(prepared_frame(0.25)).unwrap();
        port.close();
        let finisher = StreamFinisher {
            samples_tx,
            session: 7,
        };

        set_reaper_spawn_failure_for_test();
        let started = Instant::now();
        let status =
            stop_after_capture_for_test(&mut adapter, &finisher, Duration::from_millis(25));

        assert_eq!(status, StreamFinishStatus::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(adapter.retains_cleanup_ownership_for_test());
        assert!(matches!(
            samples_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            StreamMessage::Samples { .. }
        ));
        assert!(matches!(
            samples_rx.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        completion_gate.wait();
        adapter.cancel_and_join().unwrap();
    }

    #[test]
    fn two_capture_sessions_use_fresh_asr_ports_and_finish_each_once_in_fifo_order() {
        let (samples_tx, samples_rx) = mpsc::sync_channel(8);
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let delivered_for_worker = Arc::clone(&delivered);
        let recognizer = std::thread::spawn(move || {
            let mut finishes = 0;
            while finishes < 2 {
                match samples_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
                    StreamMessage::Samples { session, samples } => {
                        delivered_for_worker
                            .lock()
                            .unwrap()
                            .push((session, samples));
                    }
                    StreamMessage::Finish { session, done } => {
                        delivered_for_worker
                            .lock()
                            .unwrap()
                            .push((session, Vec::new()));
                        finishes += 1;
                        done.send(StreamFinishStatus::Completed).unwrap();
                    }
                }
            }
        });

        let mut first = SessionAsrAdapter::start(samples_tx.clone(), 1);
        let first_port = first.sink();
        first_port.try_send(prepared_frame(0.25)).unwrap();
        first_port.close();
        first.join_after_capture().unwrap();
        assert_eq!(
            StreamFinisher {
                samples_tx: samples_tx.clone(),
                session: 1,
            }
            .finish_session(),
            StreamFinishStatus::Completed
        );
        assert_eq!(first_port.outcome().accepted_frames, 1);
        assert_eq!(first_port.outcome().dropped_frames, 0);
        assert_eq!(first_port.outcome().error, None);

        let mut second = SessionAsrAdapter::start(samples_tx.clone(), 2);
        let second_port = second.sink();
        assert!(matches!(
            first_port.try_send(prepared_frame(0.5)),
            Err(crate::audio::coordinator::SinkSendError::Closed)
        ));
        second_port.try_send(prepared_frame(0.75)).unwrap();
        second_port.close();
        second.join_after_capture().unwrap();
        assert_eq!(
            StreamFinisher {
                samples_tx,
                session: 2,
            }
            .finish_session(),
            StreamFinishStatus::Completed
        );
        assert_eq!(second_port.outcome().accepted_frames, 1);
        assert_eq!(second_port.outcome().dropped_frames, 0);
        assert_eq!(second_port.outcome().error, None);

        recognizer.join().unwrap();
        assert_eq!(
            *delivered.lock().unwrap(),
            vec![
                (1, vec![0.25]),
                (1, Vec::new()),
                (2, vec![0.75]),
                (2, Vec::new()),
            ]
        );
    }

    #[test]
    fn stop_finalizes_before_a_concurrent_start_activates_the_next_session() {
        let lifecycle = Arc::new(LifecycleGate::new());
        let (samples_tx, samples_rx) = mpsc::sync_channel(8);
        let (old_adapter_drained_tx, old_adapter_drained_rx) = mpsc::channel();
        let (allow_old_finish_tx, allow_old_finish_rx) = mpsc::channel();
        let (old_finish_acked_tx, old_finish_acked_rx) = mpsc::channel();
        let (new_start_attempted_tx, new_start_attempted_rx) = mpsc::channel();
        let (new_start_waiting_tx, new_start_waiting_rx) = mpsc::channel();
        let (new_start_complete_tx, new_start_complete_rx) = mpsc::channel();
        let finalized = Arc::new(Mutex::new(Vec::new()));
        let finalized_for_worker = Arc::clone(&finalized);
        let recognizer = std::thread::spawn(move || {
            let mut expected_session = 1;
            while expected_session <= 2 {
                match samples_rx.recv_timeout(Duration::from_secs(1)).unwrap() {
                    StreamMessage::Samples { session, .. } => {
                        assert_eq!(session, expected_session);
                    }
                    StreamMessage::Finish { session, done } => {
                        assert_eq!(session, expected_session);
                        finalized_for_worker.lock().unwrap().push(session);
                        done.send(StreamFinishStatus::Completed).unwrap();
                        expected_session += 1;
                    }
                }
            }
        });

        let mut old_adapter = SessionAsrAdapter::start(samples_tx.clone(), 1);
        let old_port = old_adapter.sink();
        old_port.try_send(prepared_frame(0.25)).unwrap();
        old_port.close();

        let stop_lifecycle = Arc::clone(&lifecycle);
        let stop_samples_tx = samples_tx.clone();
        let stopper = std::thread::spawn(move || {
            let _stop = stop_lifecycle.begin_stop();
            old_adapter.join_after_capture().unwrap();
            old_adapter_drained_tx.send(()).unwrap();
            allow_old_finish_rx.recv().unwrap();
            let status = StreamFinisher {
                samples_tx: stop_samples_tx,
                session: 1,
            }
            .finish_session();
            assert_eq!(status, StreamFinishStatus::Completed);
            old_finish_acked_tx.send(()).unwrap();
        });

        old_adapter_drained_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let start_lifecycle = Arc::clone(&lifecycle);
        let new_samples_tx = samples_tx;
        let starter = std::thread::spawn(move || {
            new_start_attempted_tx.send(()).unwrap();
            let _start = start_lifecycle.begin_start_with_wait_hook(|| {
                new_start_waiting_tx.send(()).unwrap();
            });
            let mut new_adapter = SessionAsrAdapter::start(new_samples_tx.clone(), 2);
            let new_port = new_adapter.sink();
            new_port.try_send(prepared_frame(0.75)).unwrap();
            new_port.close();
            new_adapter.join_after_capture().unwrap();
            assert_eq!(
                StreamFinisher {
                    samples_tx: new_samples_tx,
                    session: 2,
                }
                .finish_session(),
                StreamFinishStatus::Completed
            );
            new_start_complete_tx.send(()).unwrap();
        });

        new_start_attempted_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        new_start_waiting_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        allow_old_finish_tx.send(()).unwrap();
        old_finish_acked_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        new_start_complete_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        stopper.join().unwrap();
        starter.join().unwrap();
        recognizer.join().unwrap();
        assert_eq!(*finalized.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn stop_tail_silence_covers_final_silence_window() {
        assert_eq!(stream::silence_samples(Duration::from_millis(1500)), 24_000);
    }

    #[test]
    fn stream_finisher_reports_backed_up_channel() {
        let (samples_tx, _samples_rx) = mpsc::sync_channel(0);
        let finisher = StreamFinisher {
            samples_tx,
            session: 1,
        };

        let status = finisher.finish_session();

        assert_eq!(status, StreamFinishStatus::BackedUp);
        assert!(status.should_retire_stream());
        assert!(status.should_report());
    }

    #[test]
    fn stream_finisher_waits_briefly_for_queue_space() {
        let (samples_tx, samples_rx) = mpsc::sync_channel(1);
        samples_tx
            .try_send(StreamMessage::Samples {
                session: 42,
                samples: vec![1.0],
            })
            .unwrap();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            match samples_rx.recv().unwrap() {
                StreamMessage::Samples { session, .. } => assert_eq!(session, 42),
                StreamMessage::Finish { .. } => panic!("expected queued samples first"),
            }
            match samples_rx.recv().unwrap() {
                StreamMessage::Finish { session, done } => {
                    assert_eq!(session, 42);
                    done.send(StreamFinishStatus::Completed).unwrap();
                }
                StreamMessage::Samples { .. } => panic!("expected finish message"),
            }
        });
        let finisher = StreamFinisher {
            samples_tx,
            session: 42,
        };

        let status = finisher.finish_session();

        assert_eq!(status, StreamFinishStatus::Completed);
        assert!(!status.should_retire_stream());
        worker.join().unwrap();
    }

    #[test]
    fn stream_finisher_reports_completed_channel() {
        let (samples_tx, samples_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || match samples_rx.recv().unwrap() {
            StreamMessage::Finish { session, done } => {
                assert_eq!(session, 42);
                done.send(StreamFinishStatus::Completed).unwrap();
            }
            StreamMessage::Samples { .. } => panic!("expected finish message"),
        });
        let finisher = StreamFinisher {
            samples_tx,
            session: 42,
        };

        let status = finisher.finish_session();

        assert_eq!(status, StreamFinishStatus::Completed);
        assert!(!status.should_retire_stream());
        assert!(!status.should_report());
        worker.join().unwrap();
    }

    #[test]
    fn stream_finisher_reports_disconnected_channel() {
        let (samples_tx, samples_rx) = mpsc::sync_channel(1);
        drop(samples_rx);
        let finisher = StreamFinisher {
            samples_tx,
            session: 1,
        };

        let status = finisher.finish_session();

        assert_eq!(status, StreamFinishStatus::Disconnected);
        assert!(status.should_retire_stream());
        assert!(status.should_report());
    }

    fn prepared_frame(sample: f32) -> PreparedFrame {
        PreparedFrame {
            metadata: AudioFrame {
                session_id: SessionId::new("adapter-test").unwrap(),
                track_id: TrackId::new("microphone").unwrap(),
                sequence: 0,
                sample_rate_hz: 16_000,
                channels: 1,
                start_ms: 0,
                duration_ms: 1,
                sample_count: 1,
            },
            samples: Arc::from([sample]),
        }
    }
}
