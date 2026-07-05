use std::io::{BufRead, BufReader, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tauri::{Emitter, Manager};

use super::state::LiveSessionState;
use super::stream::{self, LiveStreamProcess, StreamEvent};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const LEVEL_TICK: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub struct LiveRuntime {
    inner: Arc<Mutex<LiveRuntimeInner>>,
    active_session: Arc<AtomicU64>,
}

struct LiveRuntimeInner {
    session: u64,
    capture: Option<cpal::Stream>,
    stream: Option<WarmStream>,
    audio: Option<JoinHandle<()>>,
    level: Option<JoinHandle<()>>,
    vad_segments: Vec<VadSegment>,
    last_used: Instant,
    #[cfg(test)]
    has_capture_for_test: bool,
    #[cfg(test)]
    has_stream_for_test: bool,
}

struct WarmStream {
    session: u64,
    process: LiveStreamProcess,
    pcm_tx: mpsc::SyncSender<PcmMessage>,
    cancelled: Arc<AtomicBool>,
    writer: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
}

struct PcmMessage {
    session: u64,
    bytes: Vec<u8>,
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
            inner.retire_stream();
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
            selected_device_id.as_deref(),
            session,
            Arc::clone(&self.active_session),
            stream_tx,
        );
        let (capture, level, audio) = match capture {
            Ok(capture) => capture,
            Err(error) => {
                self.active_session.store(0, Ordering::SeqCst);
                let mut inner = self.inner.lock().expect("live runtime poisoned");
                inner.retire_stream();
                return Err(error);
            }
        };
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        inner.capture = Some(capture);
        inner.audio = Some(audio);
        inner.start_level_worker(app, level, session, Arc::clone(&self.active_session));
        Ok(())
    }

    pub fn stop(&self) {
        self.active_session.store(0, Ordering::SeqCst);
        let mut inner = self.inner.lock().expect("live runtime poisoned");
        inner.stop_capture();
        inner.retire_stream();
        inner.last_used = Instant::now();
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

    pub fn handle_stream_crash(&self, app: tauri::AppHandle, session: u64, message: &str) {
        if self.active_session.load(Ordering::SeqCst) != session {
            return;
        }
        self.active_session.store(0, Ordering::SeqCst);
        {
            let mut inner = self.inner.lock().expect("live runtime poisoned");
            inner.stop_capture();
            inner.retire_stream_detached_reader();
        }
        let state = app.state::<LiveSessionState>();
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
        if self
            .stream
            .as_mut()
            .is_some_and(|stream| stream.session == session && stream.process.is_running())
        {
            return Ok(());
        }
        self.retire_stream();

        let mut process =
            stream::spawn_stream_child().map_err(|err| err.user_message().to_string())?;
        let stdin = process
            .take_stdin()
            .ok_or_else(|| "Live stream stdin is unavailable.".to_string())?;
        let stdout = process
            .take_stdout()
            .ok_or_else(|| "Live stream stdout is unavailable.".to_string())?;
        let (pcm_tx, pcm_rx) = mpsc::sync_channel::<PcmMessage>(16);
        let cancelled = Arc::new(AtomicBool::new(false));

        let writer_cancelled = Arc::clone(&cancelled);
        let writer_active_session = Arc::clone(&runtime.active_session);
        let writer_runtime = runtime.clone();
        let writer_app = app.clone();
        let writer = std::thread::spawn(move || {
            let mut stdin = stdin;
            while !writer_cancelled.load(Ordering::Relaxed) {
                match pcm_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(message) => {
                        if !should_write_pcm(
                            message.session,
                            writer_active_session.load(Ordering::SeqCst),
                            session,
                        ) {
                            continue;
                        }
                        if stdin.write_all(&message.bytes).is_err() {
                            let crash_runtime = writer_runtime.clone();
                            let crash_app = writer_app.clone();
                            let crash_session = message.session;
                            std::thread::spawn(move || {
                                crash_runtime.handle_stream_crash(
                                    crash_app,
                                    crash_session,
                                    "Live stream stopped.",
                                );
                            });
                            break;
                        }
                        let _ = stdin.flush();
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        let reader_cancelled = Arc::clone(&cancelled);
        let active_session = Arc::clone(&runtime.active_session);
        let reader_runtime = runtime.clone();
        let reader = std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if reader_cancelled.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(line) = line else {
                    break;
                };
                let Some(event) = stream::parse_stream_event(&line) else {
                    continue;
                };
                let Some(session) =
                    accepted_stream_session(active_session.load(Ordering::SeqCst), session)
                else {
                    continue;
                };
                let state = app.state::<LiveSessionState>();
                match event {
                    StreamEvent::Partial(text) => {
                        let view = state.update_partial(&text);
                        let _ = app.emit("live-session", &view);
                    }
                    StreamEvent::Final(text) => {
                        let view = state.update_final(&text);
                        let _ = app.emit("live-session", &view);
                        std::thread::sleep(Duration::from_millis(180));
                        if active_session.load(Ordering::SeqCst) == session {
                            let view = state.return_to_listening();
                            let _ = app.emit("live-session", &view);
                        }
                    }
                }
            }

            let session = active_session.load(Ordering::SeqCst);
            if session != 0 && !reader_cancelled.load(Ordering::Relaxed) {
                reader_runtime.handle_stream_crash(app, session, "Live stream stopped.");
            }
        });

        self.stream = Some(WarmStream {
            session,
            process,
            pcm_tx,
            cancelled,
            writer: Some(writer),
            reader: Some(reader),
        });
        Ok(())
    }

    fn stream_tx(&self) -> Option<mpsc::SyncSender<PcmMessage>> {
        self.stream.as_ref().map(|stream| stream.pcm_tx.clone())
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
}

impl WarmStream {
    fn shutdown(mut self, join_reader: bool) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.process.shutdown();
        drop(self.pcm_tx);
        if let Some(handle) = self.writer.take() {
            let _ = handle.join();
        }
        if join_reader {
            if let Some(handle) = self.reader.take() {
                let _ = handle.join();
            }
        }
    }
}

fn open_capture(
    selected_device_id: Option<&str>,
    session: u64,
    active_session: Arc<AtomicU64>,
    pcm_tx: mpsc::SyncSender<PcmMessage>,
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
        while let Ok(raw) = raw_rx.recv() {
            if raw.session == 0 || audio_active_session.load(Ordering::SeqCst) != raw.session {
                audio_raw_ready.store(true, Ordering::Release);
                continue;
            }
            let mono = downmix_to_mono(&raw.samples, channels);
            let level = rms_level(&mono);
            let resampled = resampler.push(&mono);
            let bytes = f32_to_i16_le_bytes(&resampled);
            if !bytes.is_empty() {
                let _ = pcm_tx.try_send(PcmMessage {
                    session: raw.session,
                    bytes,
                });
            }
            let _ = level_tx.send(level);
            audio_raw_ready.store(true, Ordering::Release);
        }
    });
    let err_fn = |err| crate::stt::log_yap(&format!("live input stream error: {err}"));
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_capture_stream::<f32>(
            &device,
            &stream_config,
            session,
            Arc::clone(&active_session),
            Arc::clone(&raw_ready),
            raw_tx,
            err_fn,
        ),
        cpal::SampleFormat::I16 => build_capture_stream::<i16>(
            &device,
            &stream_config,
            session,
            Arc::clone(&active_session),
            Arc::clone(&raw_ready),
            raw_tx,
            err_fn,
        ),
        cpal::SampleFormat::U16 => build_capture_stream::<u16>(
            &device,
            &stream_config,
            session,
            Arc::clone(&active_session),
            Arc::clone(&raw_ready),
            raw_tx,
            err_fn,
        ),
        sample_format => Err(format!("Unsupported microphone format: {sample_format}")),
    }?;
    stream
        .play()
        .map_err(|err| format!("Microphone access failed: {err}"))?;
    Ok((stream, level_rx, audio))
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

fn should_write_pcm(message_session: u64, active_session: u64, stream_session: u64) -> bool {
    message_session != 0 && message_session == active_session && message_session == stream_session
}

fn accepted_stream_session(active_session: u64, stream_session: u64) -> Option<u64> {
    (active_session != 0 && active_session == stream_session).then_some(active_session)
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

pub fn downmix_to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

pub fn f32_to_i16_le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let value = if *sample <= -1.0 {
            i16::MIN
        } else if *sample >= 1.0 {
            i16::MAX
        } else {
            (*sample * i16::MAX as f32).round() as i16
        };
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum = samples.iter().map(|sample| sample * sample).sum::<f32>();
    (sum / samples.len() as f32).sqrt().clamp(0.0, 1.0)
}

pub struct LinearResampler {
    source_rate: u32,
    target_rate: u32,
    cursor: f64,
}

impl LinearResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        Self {
            source_rate: source_rate.max(1),
            target_rate: target_rate.max(1),
            cursor: 0.0,
        }
    }

    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if self.source_rate == self.target_rate {
            return input.to_vec();
        }
        let step = self.source_rate as f64 / self.target_rate as f64;
        let mut output = Vec::new();
        while self.cursor < input.len() as f64 {
            let base = self.cursor.floor() as usize;
            let frac = (self.cursor - base as f64) as f32;
            let a = input[base];
            let b = input.get(base + 1).copied().unwrap_or(a);
            output.push(a + (b - a) * frac);
            self.cursor += step;
        }
        self.cursor -= input.len() as f64;
        output
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
    fn mono_downmix_averages_channels() {
        assert_eq!(downmix_to_mono(&[1.0, 3.0, 2.0, 4.0], 2), vec![2.0, 3.0]);
    }

    #[test]
    fn pcm_conversion_clamps_to_i16() {
        assert_eq!(
            f32_to_i16_le_bytes(&[-2.0, 0.0, 2.0]),
            vec![0, 128, 0, 0, 255, 127]
        );
    }

    #[test]
    fn linear_resample_can_downsample() {
        let mut resampler = LinearResampler::new(4, 2);
        assert_eq!(resampler.push(&[0.0, 1.0, 0.0, -1.0]), vec![0.0, 0.0]);
    }

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
        assert!(should_write_pcm(2, 2, 2));
        assert!(!should_write_pcm(1, 2, 2));
        assert!(!should_write_pcm(2, 0, 2));
        assert!(!should_write_pcm(2, 2, 0));
    }

    #[test]
    fn stream_events_require_active_accepted_session() {
        assert_eq!(accepted_stream_session(3, 3), Some(3));
        assert_eq!(accepted_stream_session(0, 3), None);
        assert_eq!(accepted_stream_session(3, 0), None);
        assert_eq!(accepted_stream_session(3, 2), None);
    }

    #[test]
    fn delayed_stale_stream_events_do_not_match_new_session() {
        let old_stream_session = 4;
        let new_active_session = 5;

        assert_eq!(
            accepted_stream_session(new_active_session, old_stream_session),
            None
        );
    }

    #[test]
    fn audio_callback_claims_only_one_raw_slot() {
        let ready = AtomicBool::new(true);

        assert!(claim_raw_audio_slot(&ready));
        assert!(!claim_raw_audio_slot(&ready));
        ready.store(true, Ordering::Release);
        assert!(claim_raw_audio_slot(&ready));
    }
}
