use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tauri::{Emitter, Manager};

use crate::audio::preprocess::{
    downmix_to_mono, f32_to_i16_le_bytes, rms_level,
    AudioLevelNormalizer as LiveAudioLevelNormalizer, LinearResampler,
};

use super::state::LiveSessionState;
use super::stream::{self, LiveStreamEngine};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const LEVEL_TICK: Duration = Duration::from_millis(50);
const STREAM_FINISH_ENQUEUE_TIMEOUT: Duration = Duration::from_millis(250);
const STREAM_DRAIN_ON_STOP: Duration = Duration::from_millis(6000);

#[derive(Clone)]
pub struct LiveRuntime {
    inner: Arc<Mutex<LiveRuntimeInner>>,
    active_session: Arc<AtomicU64>,
    recorded_pcm: Arc<Mutex<Vec<u8>>>,
}

struct LiveRuntimeInner {
    session: u64,
    capture: Option<cpal::Stream>,
    stream: Option<SessionStream>,
    audio: Option<JoinHandle<()>>,
    level: Option<JoinHandle<()>>,
    vad_segments: Vec<VadSegment>,
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

struct RawAudio {
    session: u64,
    samples: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VadSegment {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl LiveRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(LiveRuntimeInner::new())),
            active_session: Arc::new(AtomicU64::new(0)),
            recorded_pcm: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn is_active(&self) -> bool {
        self.inner
            .lock()
            .expect("live runtime poisoned")
            .capture
            .is_some()
    }

    pub fn start_local(
        &self,
        app: tauri::AppHandle,
        selected_device_id: Option<String>,
    ) -> Result<(), String> {
        let (session, stream_tx) = {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            if inner.capture.is_some() {
                return Err("Live capture is already running.".into());
            }
            self.discard_recorded_pcm();
            inner.session = inner.session.saturating_add(1);
            inner.last_used = Instant::now();
            inner.vad_segments.clear();
            let session = inner.session;
            inner.ensure_stream(self.clone(), app.clone(), session)?;
            let stream_tx = inner
                .stream_tx()
                .ok_or_else(|| "Live stream is unavailable.".to_string())?;
            (session, stream_tx)
        };

        let state = app.state::<LiveSessionState>();
        let view = state.clear_for_new_session();
        let _ = app.emit("live-session", &view);

        self.active_session.store(session, Ordering::SeqCst);

        let capture = open_capture(
            self.clone(),
            app.clone(),
            selected_device_id.as_deref(),
            session,
            Arc::clone(&self.active_session),
            stream_tx,
            Arc::clone(&self.recorded_pcm),
        );
        let (capture, level, audio) = match capture {
            Ok(capture) => capture,
            Err(error) => {
                self.active_session.store(0, Ordering::SeqCst);
                return Err(error);
            }
        };
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        inner.capture = Some(capture);
        inner.audio = Some(audio);
        inner.start_level_worker(app, level, session, Arc::clone(&self.active_session));
        Ok(())
    }

    pub fn warm(&self, app: tauri::AppHandle) -> Result<(), String> {
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        let session = inner.session;
        inner.ensure_stream(self.clone(), app, session)
    }

    pub fn stop(&self) -> StreamFinishStatus {
        let finisher = {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            inner.stop_capture();
            inner.stream_finisher()
        };
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
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        inner.stop_capture();
        inner.retire_stream();
        self.active_session.store(0, Ordering::SeqCst);
    }

    pub fn take_recorded_pcm(&self) -> Vec<u8> {
        std::mem::take(&mut *self.recorded_pcm.lock().expect("live pcm poisoned"))
    }

    fn discard_recorded_pcm(&self) {
        let _ = self.take_recorded_pcm();
    }

    pub fn handle_stream_crash(&self, app: tauri::AppHandle, session: u64, message: &str) {
        if self.active_session.load(Ordering::SeqCst) != session {
            return;
        }
        let state = app.state::<LiveSessionState>();
        let before_crash = state.snapshot();
        self.active_session.store(0, Ordering::SeqCst);
        {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            inner.stop_capture();
            inner.retire_stream_detached_reader();
        }
        app.state::<crate::runtime::RuntimeOrchestratorState>()
            .with(|orchestrator| orchestrator.finish_active_work());
        match super::recordings::save_session_files(self, &before_crash) {
            Ok(Some(saved)) => {
                let _ = app.emit("live-session-saved", &saved);
            }
            Ok(None) => {}
            Err(error) => crate::stt::log_yap(&format!("live crash save failed: {error}")),
        }
        let view = state.block_with_error(message);
        let _ = app.emit("live-session", &view);
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
            audio: None,
            level: None,
            vad_segments: Vec::new(),
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
            let _ = handle.join();
        }
        let handle = std::thread::spawn(move || {
            let state = app.state::<LiveSessionState>();
            while let Ok(first) = level.recv() {
                let mut value = first;
                while let Ok(next) = level.try_recv() {
                    value = next;
                }
                if active_session.load(Ordering::SeqCst) != session {
                    break;
                }
                let view = state.update_level(value);
                let _ = app.emit("live-session", &view);
                std::thread::sleep(LEVEL_TICK);
            }
        });
        self.level = Some(handle);
    }

    fn stop_capture(&mut self) {
        self.capture.take();
        if let Some(handle) = self.audio.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.level.take() {
            let _ = handle.join();
        }
        #[cfg(test)]
        {
            self.has_capture_for_test = false;
        }
    }

    fn retire_stream(&mut self) {
        if let Some(stream) = self.stream.take() {
            stream.shutdown(true);
        }
        #[cfg(test)]
        {
            self.has_stream_for_test = false;
        }
    }

    fn retire_stream_detached_reader(&mut self) {
        if let Some(stream) = self.stream.take() {
            stream.shutdown(false);
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

    fn shutdown(mut self, join_reader: bool) {
        self.cancelled.store(true, Ordering::SeqCst);
        drop(self.samples_tx);
        if join_reader {
            if let Some(handle) = self.worker.take() {
                let _ = handle.join();
            }
        }
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

fn open_capture(
    runtime: LiveRuntime,
    app: tauri::AppHandle,
    selected_device_id: Option<&str>,
    session: u64,
    active_session: Arc<AtomicU64>,
    samples_tx: mpsc::SyncSender<StreamMessage>,
    recorded_pcm: Arc<Mutex<Vec<u8>>>,
) -> Result<(cpal::Stream, mpsc::Receiver<f32>, JoinHandle<()>), String> {
    let host = cpal::default_host();
    let device = resolve_capture_device(&host, selected_device_id)
        .ok_or_else(|| "No input detected.".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|err| format!("Microphone access failed: {err}"))?;
    let channels = usize::from(config.channels());
    let sample_rate = config.sample_rate().0;
    let stream_config = config.config();
    let (raw_tx, raw_rx) = mpsc::sync_channel::<RawAudio>(8);
    let raw_ready = Arc::new(AtomicBool::new(true));
    let (level_tx, level_rx) = mpsc::channel::<f32>();
    let audio_active_session = Arc::clone(&active_session);
    let audio_raw_ready = Arc::clone(&raw_ready);
    let audio = std::thread::spawn(move || {
        let mut resampler = LinearResampler::new(sample_rate, TARGET_SAMPLE_RATE);
        let mut level_normalizer = LiveAudioLevelNormalizer::new();
        while let Ok(raw) = raw_rx.recv() {
            if raw.session == 0 || audio_active_session.load(Ordering::SeqCst) != raw.session {
                audio_raw_ready.store(true, Ordering::Release);
                continue;
            }
            let mono = downmix_to_mono(&raw.samples, channels);
            let level = level_normalizer.normalized_level(rms_level(&mono));
            let resampled = resampler.push(&mono);
            let bytes = f32_to_i16_le_bytes(&resampled);
            if !bytes.is_empty() {
                // Keep live WAV data in memory for short dictation sessions.
                recorded_pcm
                    .lock()
                    .expect("live pcm poisoned")
                    .extend_from_slice(&bytes);
                // ponytail: if the decoder falls behind, keep the WAV and drop the live chunk.
                let _ = samples_tx.try_send(StreamMessage::Samples {
                    session: raw.session,
                    samples: resampled,
                });
            }
            let _ = level_tx.send(level);
            audio_raw_ready.store(true, Ordering::Release);
        }
    });
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_capture_stream::<f32>(
            &device,
            &stream_config,
            session,
            Arc::clone(&active_session),
            Arc::clone(&raw_ready),
            raw_tx,
            capture_error_handler(runtime.clone(), app.clone(), session),
        ),
        cpal::SampleFormat::I16 => build_capture_stream::<i16>(
            &device,
            &stream_config,
            session,
            Arc::clone(&active_session),
            Arc::clone(&raw_ready),
            raw_tx,
            capture_error_handler(runtime.clone(), app.clone(), session),
        ),
        cpal::SampleFormat::U16 => build_capture_stream::<u16>(
            &device,
            &stream_config,
            session,
            Arc::clone(&active_session),
            Arc::clone(&raw_ready),
            raw_tx,
            capture_error_handler(runtime, app, session),
        ),
        sample_format => Err(format!("Unsupported microphone format: {sample_format}")),
    }?;
    stream
        .play()
        .map_err(|err| format!("Microphone access failed: {err}"))?;
    Ok((stream, level_rx, audio))
}

fn capture_error_handler(
    runtime: LiveRuntime,
    app: tauri::AppHandle,
    session: u64,
) -> impl FnMut(cpal::StreamError) + Send + 'static {
    move |err| {
        let message = format!("Microphone input stopped: {err}");
        crate::stt::log_yap(&format!("live input stream error: {err}"));
        let runtime = runtime.clone();
        let app = app.clone();
        std::thread::spawn(move || runtime.handle_stream_crash(app, session, &message));
    }
}

fn build_capture_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    session: u64,
    active_session: Arc<AtomicU64>,
    raw_ready: Arc<AtomicBool>,
    raw_tx: mpsc::SyncSender<RawAudio>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + SampleToF32,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if active_session.load(Ordering::SeqCst) != session {
                    return;
                }
                if !claim_raw_audio_slot(&raw_ready) {
                    return;
                }
                let samples = data.iter().map(SampleToF32::to_f32).collect::<Vec<_>>();
                if raw_tx.try_send(RawAudio { session, samples }).is_err() {
                    raw_ready.store(true, Ordering::Release);
                }
            },
            err_fn,
            Some(Duration::from_millis(250)),
        )
        .map_err(|err| format!("Microphone access failed: {err}"))
}

fn claim_raw_audio_slot(raw_ready: &AtomicBool) -> bool {
    raw_ready
        .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
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
    if app
        .state::<LiveRuntime>()
        .active_session
        .load(Ordering::SeqCst)
        != session
    {
        return;
    }
    let state = app.state::<LiveSessionState>();
    let view = state.update_partial(text);
    let _ = app.emit("live-session", &view);
}

fn emit_stream_final(app: &tauri::AppHandle, session: u64, text: &str) {
    if app
        .state::<LiveRuntime>()
        .active_session
        .load(Ordering::SeqCst)
        != session
    {
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
    message_session != 0 && message_session == active_session && message_session == stream_session
}

fn resolve_capture_device(host: &cpal::Host, selected_id: Option<&str>) -> Option<cpal::Device> {
    let mut first = None;
    let mut default = None;
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let devices = host.input_devices().ok()?;
    for (index, device) in devices.enumerate() {
        let name = device.name().ok()?;
        let id = format!("{index}:{name}");
        if selected_id.is_some_and(|selected| selected == id) {
            return Some(device);
        }
        if first.is_none() {
            first = Some(device.clone());
        }
        if default.is_none() && default_name.as_deref() == Some(name.as_str()) {
            default = Some(device);
        }
    }
    default.or(first)
}

trait SampleToF32 {
    fn to_f32(&self) -> f32;
}

impl SampleToF32 for f32 {
    fn to_f32(&self) -> f32 {
        *self
    }
}

impl SampleToF32 for i16 {
    fn to_f32(&self) -> f32 {
        (*self as f32 / i16::MAX as f32).clamp(-1.0, 1.0)
    }
}

impl SampleToF32 for u16 {
    fn to_f32(&self) -> f32 {
        ((*self as f32 - 32_768.0) / 32_768.0).clamp(-1.0, 1.0)
    }
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
    fn stale_pcm_is_discarded_after_session_changes() {
        assert!(should_accept_stream_samples(2, 2, 2));
        assert!(!should_accept_stream_samples(1, 2, 2));
        assert!(!should_accept_stream_samples(2, 0, 2));
        assert!(!should_accept_stream_samples(2, 2, 0));
    }

    #[test]
    fn audio_callback_claims_only_one_raw_slot() {
        let ready = AtomicBool::new(true);

        assert!(claim_raw_audio_slot(&ready));
        assert!(!claim_raw_audio_slot(&ready));
        ready.store(true, Ordering::Release);
        assert!(claim_raw_audio_slot(&ready));
    }

    #[test]
    fn taking_recorded_pcm_releases_buffer_without_clone() {
        let runtime = LiveRuntime::new();
        {
            let mut pcm = runtime.recorded_pcm.lock().unwrap();
            pcm.reserve(1024);
            pcm.extend_from_slice(&[1, 2, 3, 4]);
        }

        let pcm = runtime.take_recorded_pcm();

        assert_eq!(pcm, vec![1, 2, 3, 4]);
        let retained = runtime.recorded_pcm.lock().unwrap();
        assert!(retained.is_empty());
        assert_eq!(retained.capacity(), 0);
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
