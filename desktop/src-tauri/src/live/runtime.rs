use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

use crate::audio::capture::{CaptureAdapter, CapturePacket, CapturePorts};
use crate::audio::preprocess::{
    downmix_to_mono, f32_to_i16_le_bytes, rms_level,
    AudioLevelNormalizer as LiveAudioLevelNormalizer, LinearResampler,
};

use super::state::{LiveLevelView, LiveSessionState};
use super::stream::{self, LiveStreamEngine};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const LEVEL_TICK: Duration = Duration::from_millis(50);
const MAX_RECORDED_PCM_SECONDS: usize = 10 * 60;
const MAX_RECORDED_PCM_BYTES: usize = TARGET_SAMPLE_RATE as usize * 2 * MAX_RECORDED_PCM_SECONDS;
const STREAM_FINISH_ENQUEUE_TIMEOUT: Duration = Duration::from_millis(250);
const STREAM_DRAIN_ON_STOP: Duration = Duration::from_millis(6000);
const CRASH_CLAIM_BIT: u64 = 1 << 63;

fn active_session_matches(active_session: u64, session: u64) -> bool {
    session != 0 && (active_session == session || active_session == session | CRASH_CLAIM_BIT)
}

#[derive(Clone)]
pub struct LiveRuntime {
    inner: Arc<Mutex<LiveRuntimeInner>>,
    active_session: Arc<AtomicU64>,
    recorded_pcm: Arc<Mutex<RecordedPcmBuffer>>,
    transition: Arc<Mutex<()>>,
    warming: Arc<AtomicBool>,
}

struct LiveRuntimeInner {
    session: u64,
    capture: Option<CaptureAdapter>,
    stream: Option<SessionStream>,
    level: Option<JoinHandle<()>>,
    last_used: Instant,
    #[cfg(test)]
    has_capture_for_test: bool,
    #[cfg(test)]
    has_stream_for_test: bool,
}

struct SessionStream {
    session: Arc<AtomicU64>,
    samples_tx: mpsc::SyncSender<StreamMessage>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

enum StreamMessage {
    Samples {
        session: u64,
        samples: Vec<f32>,
    },
    Finish {
        session: u64,
        done: mpsc::Sender<()>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum StreamSendStatus {
    Sent,
    BackedUp,
    Disconnected,
}

#[derive(Debug, PartialEq, Eq)]
enum RecordedPcmAppendStatus {
    Stored,
    Capped,
}

struct RecordedPcmBuffer {
    bytes: Vec<u8>,
    capped: bool,
    max_bytes: usize,
}

pub(crate) struct RecordedPcm {
    pub(crate) bytes: Vec<u8>,
    pub(crate) capped: bool,
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

impl RecordedPcmBuffer {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            capped: false,
            max_bytes: MAX_RECORDED_PCM_BYTES,
        }
    }

    #[cfg(test)]
    fn with_limit_for_test(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::new(),
            capped: false,
            max_bytes,
        }
    }

    fn append(&mut self, bytes: &[u8]) -> RecordedPcmAppendStatus {
        if bytes.is_empty() {
            return RecordedPcmAppendStatus::Stored;
        }
        let remaining = self.max_bytes.saturating_sub(self.bytes.len());
        if remaining == 0 {
            self.capped = true;
            return RecordedPcmAppendStatus::Capped;
        }
        let accepted = remaining.min(bytes.len()) & !1;
        if accepted == 0 {
            self.capped = true;
            return RecordedPcmAppendStatus::Capped;
        }
        if self.bytes.try_reserve(accepted).is_err() {
            self.capped = true;
            return RecordedPcmAppendStatus::Capped;
        }
        self.bytes.extend_from_slice(&bytes[..accepted]);
        if accepted < bytes.len() {
            self.capped = true;
            RecordedPcmAppendStatus::Capped
        } else {
            RecordedPcmAppendStatus::Stored
        }
    }

    fn take(&mut self) -> RecordedPcm {
        let capped = self.capped;
        self.capped = false;
        RecordedPcm {
            bytes: std::mem::take(&mut self.bytes),
            capped,
        }
    }

    fn restore(&mut self, pcm: RecordedPcm) {
        if pcm.bytes.is_empty() {
            return;
        }
        self.bytes = pcm.bytes;
        self.capped = pcm.capped;
    }

    #[cfg(test)]
    fn reserve(&mut self, additional: usize) {
        self.bytes.reserve(additional);
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.bytes.capacity()
    }

    #[cfg(test)]
    fn was_capped(&self) -> bool {
        self.capped
    }
}

impl LiveRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LiveRuntimeInner::new())),
            active_session: Arc::new(AtomicU64::new(0)),
            recorded_pcm: Arc::new(Mutex::new(RecordedPcmBuffer::new())),
            transition: Arc::new(Mutex::new(())),
            warming: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn transition_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.transition
            .lock()
            .expect("live transition gate poisoned")
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
    ) -> Result<(), LiveStartFailure> {
        let (session, stream_tx) = {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if inner.capture.is_some() {
                return Ok(());
            }
            self.discard_recorded_pcm();
            inner.session = inner.session.saturating_add(1);
            inner.last_used = Instant::now();
            let session = inner.session;
            self.active_session.store(session, Ordering::SeqCst);
            if let Err(message) = inner.ensure_stream(self.clone(), app.clone(), session) {
                return Err(LiveStartFailure::new(session, message));
            }
            let Some(stream_tx) = inner.stream_tx() else {
                return Err(LiveStartFailure::new(
                    session,
                    "Live stream is unavailable.".into(),
                ));
            };
            (session, stream_tx)
        };

        let state = app.state::<LiveSessionState>();
        let Some(view) = state.try_begin_listening_from_armed() else {
            let _ = self.active_session.compare_exchange(
                session,
                0,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            return Ok(());
        };
        let _ = app.emit("live-session", &view);

        let resolved = super::devices::resolve_capture_device(selected_device_id.as_deref())
            .map_err(|error| LiveStartFailure::new(session, error))?;
        let stream_config = resolved.config.config();
        let sample_format = resolved.config.sample_format();
        let (level_tx, level) = mpsc::channel::<f32>();
        let capture_runtime = self.clone();
        let capture_app = app.clone();
        let capture_active_session = Arc::clone(&self.active_session);
        let capture_recorded_pcm = Arc::clone(&self.recorded_pcm);
        let capture = CaptureAdapter::open(
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
                        samples_tx: stream_tx,
                        recorded_pcm: capture_recorded_pcm,
                        level_tx,
                    },
                );
            },
        )
        .map_err(|error| LiveStartFailure::new(session, error))?;
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
            drop(level);
            return Ok(());
        }
        inner.capture = Some(capture);
        inner.start_level_worker(app, level, session, Arc::clone(&self.active_session));
        Ok(())
    }

    pub fn warm(&self, app: tauri::AppHandle) -> Result<(), String> {
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

    pub fn stop(&self) -> StreamFinishStatus {
        let (finisher, shutdown_errors) = {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            let shutdown_errors = inner.stop_capture();
            (inner.stream_finisher(), shutdown_errors)
        };
        log_worker_shutdown_errors(shutdown_errors);
        let finish_status = finisher
            .as_ref()
            .map(StreamFinisher::finish_session)
            .unwrap_or(StreamFinishStatus::NoStream);
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if finish_status.should_retire_stream() {
            inner.retire_stream_detached_reader();
        }
        self.active_session.store(0, Ordering::SeqCst);
        inner.last_used = Instant::now();
        finish_status
    }

    pub fn unload_if_idle(&self, threshold: Duration) {
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        if inner.capture.is_none() && inner.last_used.elapsed() >= threshold {
            inner.retire_stream();
        }
    }

    pub fn shutdown(&self) {
        let _transition = self.transition_guard();
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        let shutdown_errors = inner.stop_capture();
        inner.retire_stream();
        self.active_session.store(0, Ordering::SeqCst);
        drop(inner);
        log_worker_shutdown_errors(shutdown_errors);
    }

    pub(crate) fn take_recorded_pcm(&self) -> RecordedPcm {
        self.recorded_pcm.lock().expect("live pcm poisoned").take()
    }

    pub(crate) fn restore_recorded_pcm(&self, pcm: RecordedPcm) {
        self.recorded_pcm
            .lock()
            .expect("live pcm poisoned")
            .restore(pcm);
    }

    #[cfg(test)]
    pub(crate) fn append_recorded_pcm_for_test(&self, bytes: &[u8]) {
        self.recorded_pcm
            .lock()
            .expect("live pcm poisoned")
            .append(bytes);
    }

    #[cfg(test)]
    pub(crate) fn set_recorded_pcm_limit_for_test(&self, max_bytes: usize) {
        self.recorded_pcm
            .lock()
            .expect("live pcm poisoned")
            .max_bytes = max_bytes;
    }

    fn discard_recorded_pcm(&self) {
        let _ = self.take_recorded_pcm();
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
        let (samples_tx, samples_rx) = mpsc::sync_channel::<StreamMessage>(64);
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

    fn stream_tx(&self) -> Option<mpsc::SyncSender<StreamMessage>> {
        self.stream.as_ref().map(|stream| stream.samples_tx.clone())
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

    fn stop_capture(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Some(capture) = self.capture.take() {
            if let Err(error) = capture.shutdown() {
                errors.push(error);
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
        errors
    }

    fn retire_stream(&mut self) {
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
                        Ok(()) => StreamFinishStatus::Completed,
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
    samples_tx: mpsc::SyncSender<StreamMessage>,
    recorded_pcm: Arc<Mutex<RecordedPcmBuffer>>,
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
        samples_tx,
        recorded_pcm,
        level_tx,
    } = context;
    let packet_runtime = runtime.clone();
    let packet_app = app.clone();
    let error_runtime = runtime.clone();
    let error_app = app.clone();
    let loss_runtime = runtime.clone();
    let loss_app = app.clone();
    let mut resampler = None;
    let mut capture_config = None;
    let mut level_normalizer = LiveAudioLevelNormalizer::new();
    let mut decoder_backpressure_reported = false;
    let mut decoder_disconnected_reported = false;
    run_guarded_capture_packet_worker(
        || {
            run_capture_packet_loop(
                ports,
                errors,
                move |packet| {
                    if !active_session_matches(active_session.load(Ordering::SeqCst), session) {
                        return false;
                    }
                    let config = (packet.channels, packet.sample_rate_hz);
                    if packet.channels == 0
                        || packet.sample_rate_hz == 0
                        || capture_config.is_some_and(|current| current != config)
                    {
                        if !decoder_disconnected_reported {
                            decoder_disconnected_reported = true;
                            spawn_stream_crash_handler(
                                packet_app.clone(),
                                packet_runtime.clone(),
                                session,
                                "Microphone input configuration changed unexpectedly.".to_string(),
                            );
                        }
                        return true;
                    }
                    capture_config.get_or_insert(config);
                    let resampler = resampler.get_or_insert_with(|| {
                        LinearResampler::new(packet.sample_rate_hz, TARGET_SAMPLE_RATE)
                    });
                    let mono = downmix_to_mono(&packet.samples, usize::from(packet.channels));
                    let level = level_normalizer.normalized_level(rms_level(&mono));
                    let resampled = resampler.push(&mono);
                    let bytes = f32_to_i16_le_bytes(&resampled);
                    if !bytes.is_empty() {
                        match recorded_pcm.lock() {
                            Ok(mut recorded_pcm) => {
                                recorded_pcm.append(&bytes);
                            }
                            Err(_) => {
                                spawn_stream_crash_handler(
                                    packet_app.clone(),
                                    packet_runtime.clone(),
                                    session,
                                    "Live recording buffer became unavailable.".to_string(),
                                );
                                return true;
                            }
                        }
                        match try_send_stream_samples(&samples_tx, session, resampled) {
                            StreamSendStatus::Sent => {}
                            StreamSendStatus::BackedUp => {
                                if !decoder_backpressure_reported {
                                    decoder_backpressure_reported = true;
                                    let state = packet_app.state::<LiveSessionState>();
                                    let view = state.mark_transcription_backpressure();
                                    let _ = packet_app.emit("live-session", &view);
                                }
                            }
                            StreamSendStatus::Disconnected => {
                                if !decoder_disconnected_reported {
                                    decoder_disconnected_reported = true;
                                    spawn_stream_crash_handler(
                                        packet_app.clone(),
                                        packet_runtime.clone(),
                                        session,
                                        "Live transcription stopped unexpectedly.".to_string(),
                                    );
                                }
                                return true;
                            }
                        }
                    }
                    let _ = level_tx.send(level);
                    false
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
                    Ok(snapshot) => {
                        crate::stt::log_yap(&format!(
                            "live capture degraded: source_position_frames={} dropped_frames={} cause={:?}",
                            snapshot.first_source_position_frames,
                            snapshot.dropped_frames,
                            snapshot.cause
                        ));
                        false
                    }
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
    P: FnMut(&CapturePacket) -> bool,
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
    P: FnMut(&CapturePacket) -> bool,
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
                let should_exit = process_packet(&packet);
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

fn try_send_stream_samples(
    samples_tx: &mpsc::SyncSender<StreamMessage>,
    session: u64,
    samples: Vec<f32>,
) -> StreamSendStatus {
    match samples_tx.try_send(StreamMessage::Samples { session, samples }) {
        Ok(()) => StreamSendStatus::Sent,
        Err(mpsc::TrySendError::Full(_)) => StreamSendStatus::BackedUp,
        Err(mpsc::TrySendError::Disconnected(_)) => StreamSendStatus::Disconnected,
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
            Ok(StreamMessage::Samples { session, samples }) => {
                if !should_accept_stream_samples(
                    session,
                    active_session.load(Ordering::SeqCst),
                    stream_session.load(Ordering::SeqCst),
                ) {
                    continue;
                }
                if active_stream_session != session {
                    engine.reset();
                    buffer.clear();
                    profile = StreamProfile::new(session);
                    active_stream_session = session;
                }
                buffer.extend(samples);
                drain_stream_buffer(&mut engine, &mut buffer, &mut profile, &app, false);
            }
            Ok(StreamMessage::Finish { session, done }) => {
                if active_stream_session == session {
                    drain_stream_buffer(&mut engine, &mut buffer, &mut profile, &app, true);
                    let started = Instant::now();
                    let final_text = engine.finish();
                    profile.decode_elapsed += started.elapsed();
                    if let Some(text) = final_text {
                        emit_stream_final(&app, session, &text);
                    }
                    crate::stt::log_stt(&profile.summary());
                    engine.reset();
                    buffer.clear();
                    active_stream_session = 0;
                }
                let _ = done.send(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
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
            run_capture_packet_loop(ports, error_rx, |_| false, |_| false, |_| false);
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
                |_| false,
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
                |_| false,
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
                move |_| {
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
    fn transition_gate_serializes_start_and_stop_ownership() {
        let runtime = LiveRuntime::new();
        let first = runtime.transition_guard();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let contender = runtime.clone();
        let worker = std::thread::spawn(move || {
            let _guard = contender.transition_guard();
            acquired_tx.send(()).unwrap();
        });

        assert!(acquired_rx.recv_timeout(Duration::from_millis(25)).is_err());
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.join().unwrap();
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
    fn taking_recorded_pcm_releases_buffer_without_clone() {
        let runtime = LiveRuntime::new();
        {
            let mut pcm = runtime.recorded_pcm.lock().unwrap();
            pcm.reserve(1024);
            assert_eq!(pcm.append(&[1, 2, 3, 4]), RecordedPcmAppendStatus::Stored);
        }

        let pcm = runtime.take_recorded_pcm();

        assert_eq!(pcm.bytes, vec![1, 2, 3, 4]);
        assert!(!pcm.capped);
        let retained = runtime.recorded_pcm.lock().unwrap();
        assert!(retained.is_empty());
        assert_eq!(retained.capacity(), 0);
    }

    #[test]
    fn recorded_pcm_buffer_caps_retained_audio() {
        let mut pcm = RecordedPcmBuffer::with_limit_for_test(6);

        assert_eq!(pcm.append(&[1, 2, 3, 4]), RecordedPcmAppendStatus::Stored);
        assert_eq!(pcm.append(&[5, 6, 7, 8]), RecordedPcmAppendStatus::Capped);

        assert!(pcm.was_capped());
        let taken = pcm.take();
        assert_eq!(taken.bytes, vec![1, 2, 3, 4, 5, 6]);
        assert!(taken.capped);
    }

    #[test]
    fn stream_sample_send_distinguishes_backpressure_from_disconnect() {
        let (full_tx, _full_rx) = mpsc::sync_channel(0);
        assert_eq!(
            try_send_stream_samples(&full_tx, 1, vec![1.0]),
            StreamSendStatus::BackedUp
        );

        let (disconnected_tx, disconnected_rx) = mpsc::sync_channel(1);
        drop(disconnected_rx);
        assert_eq!(
            try_send_stream_samples(&disconnected_tx, 1, vec![1.0]),
            StreamSendStatus::Disconnected
        );
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
                    done.send(()).unwrap();
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
                done.send(()).unwrap();
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
}
