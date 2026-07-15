use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tauri::Manager;

use super::super::state::LiveSessionState;
use super::super::stream::{self, LiveStreamEngine, StreamMessage};
use super::session_identity::active_session_matches;
use super::warmup::SharedWarmup;
use super::worker::join_worker;

const TARGET_SAMPLE_RATE: u32 = 16_000;
const FINISH_ENQUEUE_TIMEOUT: Duration = Duration::from_millis(250);
const DRAIN_ON_STOP: Duration = Duration::from_millis(6000);

pub(super) struct SessionStream {
    session: Arc<AtomicU64>,
    samples_tx: mpsc::SyncSender<StreamMessage>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    model_warmup: Option<Arc<SharedWarmup<LiveStreamEngine>>>,
}

pub(super) struct StreamFinisher {
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

struct StreamWorker {
    engine: LiveStreamEngine,
    buffer: Vec<f32>,
    profile: StreamProfile,
    app: tauri::AppHandle,
    active_session: Arc<AtomicU64>,
    stream_session: Arc<AtomicU64>,
    active_stream_session: u64,
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

impl SessionStream {
    pub(super) fn start(
        engine: LiveStreamEngine,
        session: u64,
        active_session: Arc<AtomicU64>,
        app: tauri::AppHandle,
        model_warmup: Arc<SharedWarmup<LiveStreamEngine>>,
    ) -> Self {
        let (samples_tx, samples_rx) = mpsc::sync_channel::<StreamMessage>(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stream_session = Arc::new(AtomicU64::new(session));
        let worker_cancelled = Arc::clone(&cancelled);
        let worker_session = Arc::clone(&stream_session);
        let worker = std::thread::spawn(move || {
            StreamWorker::new(engine, app, active_session, worker_session)
                .run(samples_rx, worker_cancelled);
        });

        Self {
            session: stream_session,
            samples_tx,
            cancelled,
            worker: Some(worker),
            model_warmup: Some(model_warmup),
        }
    }

    pub(super) fn retarget(&self, session: u64) {
        self.session.store(session, Ordering::SeqCst);
    }

    pub(super) fn sender(&self) -> mpsc::SyncSender<StreamMessage> {
        self.samples_tx.clone()
    }

    pub(super) fn is_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }

    pub(super) fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub(super) fn cancel_reader(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(super) fn finisher(&self) -> StreamFinisher {
        StreamFinisher::new(self.samples_tx.clone(), self.session.load(Ordering::SeqCst))
    }

    pub(super) fn shutdown(mut self, join_reader: bool) -> Result<(), String> {
        self.cancelled.store(true, Ordering::SeqCst);
        drop(self.samples_tx);
        let result = if join_reader {
            if let Some(handle) = self.worker.take() {
                join_worker(handle)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };
        if let Some(warmup) = self.model_warmup.take() {
            warmup.release_in_use();
        }
        result
    }

    #[cfg(test)]
    pub(super) fn from_worker_for_test(
        session: u64,
        worker: JoinHandle<()>,
        cancelled: bool,
    ) -> Self {
        let (samples_tx, _samples_rx) = mpsc::sync_channel(1);
        Self {
            session: Arc::new(AtomicU64::new(session)),
            samples_tx,
            cancelled: Arc::new(AtomicBool::new(cancelled)),
            worker: Some(worker),
            model_warmup: None,
        }
    }
}

impl StreamFinisher {
    pub(super) fn new(samples_tx: mpsc::SyncSender<StreamMessage>, session: u64) -> Self {
        Self {
            samples_tx,
            session,
        }
    }

    pub(super) fn finish_session(&self) -> StreamFinishStatus {
        let (done_tx, done_rx) = mpsc::channel();
        let mut message = StreamMessage::Finish {
            session: self.session,
            done: done_tx,
        };
        let started = Instant::now();

        loop {
            match self.samples_tx.try_send(message) {
                Ok(()) => {
                    return match done_rx.recv_timeout(DRAIN_ON_STOP) {
                        Ok(status) => status,
                        Err(mpsc::RecvTimeoutError::Timeout) => StreamFinishStatus::TimedOut,
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            StreamFinishStatus::Disconnected
                        }
                    };
                }
                Err(mpsc::TrySendError::Full(returned)) => {
                    if started.elapsed() >= FINISH_ENQUEUE_TIMEOUT {
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

impl StreamFinishStatus {
    pub(super) fn should_retire_stream(self) -> bool {
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

impl StreamWorker {
    fn new(
        engine: LiveStreamEngine,
        app: tauri::AppHandle,
        active_session: Arc<AtomicU64>,
        stream_session: Arc<AtomicU64>,
    ) -> Self {
        Self {
            engine,
            buffer: Vec::with_capacity(stream::chunk_samples() * 2),
            profile: StreamProfile::default(),
            app,
            active_session,
            stream_session,
            active_stream_session: 0,
        }
    }

    fn run(mut self, samples_rx: mpsc::Receiver<StreamMessage>, cancelled: Arc<AtomicBool>) {
        while !cancelled.load(Ordering::Relaxed) {
            match samples_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => self.process(message),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn process(&mut self, message: StreamMessage) {
        match message {
            StreamMessage::Samples { session, samples } => {
                if !should_accept_stream_samples(
                    session,
                    self.active_session.load(Ordering::SeqCst),
                    self.stream_session.load(Ordering::SeqCst),
                ) {
                    return;
                }
                if self.active_stream_session != session {
                    self.engine.reset();
                    self.buffer.clear();
                    self.profile = StreamProfile::new(session);
                    self.active_stream_session = session;
                }
                self.buffer.extend(samples);
                self.drain_buffer(false);
            }
            StreamMessage::Finish { session, done } => {
                if self.active_stream_session == session {
                    self.drain_buffer(true);
                    let started = Instant::now();
                    let final_text = self.engine.finish();
                    self.profile.decode_elapsed += started.elapsed();
                    if let Some(text) = final_text {
                        self.emit_final(session, &text);
                    }
                    crate::stt::log_stt(&self.profile.summary());
                    self.engine.reset();
                    self.buffer.clear();
                    self.active_stream_session = 0;
                    let _ = done.send(StreamFinishStatus::Completed);
                } else {
                    let _ = done.send(StreamFinishStatus::NoStream);
                }
            }
        }
    }

    fn drain_buffer(&mut self, flush_all: bool) {
        let chunk = stream::chunk_samples();
        while self.buffer.len() >= chunk || (flush_all && !self.buffer.is_empty()) {
            let take = if self.buffer.len() >= chunk {
                chunk
            } else {
                self.buffer.len()
            };
            let samples = self.buffer.drain(..take).collect::<Vec<_>>();
            self.profile.audio_samples += samples.len();
            self.profile.chunks += 1;
            let started = Instant::now();
            let text = self.engine.accept_samples(&samples);
            self.profile.decode_elapsed += started.elapsed();
            if let Some(text) = text {
                self.profile.mark_first_text();
                self.emit_partial(self.profile.session, &text);
            }
        }
    }

    fn emit_partial(&self, session: u64, text: &str) {
        if !active_session_matches(self.active_session.load(Ordering::SeqCst), session) {
            return;
        }
        let state = self.app.state::<LiveSessionState>();
        let view = state.update_partial(text);
        super::super::events::emit_session(&self.app, &view);
    }

    fn emit_final(&self, session: u64, text: &str) {
        if !active_session_matches(self.active_session.load(Ordering::SeqCst), session) {
            return;
        }
        let state = self.app.state::<LiveSessionState>();
        let view = state.update_final(text);
        super::super::events::emit_session(&self.app, &view);
        std::thread::sleep(Duration::from_millis(180));
        let view = state.return_to_listening();
        super::super::events::emit_session(&self.app, &view);
    }
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

pub(super) fn should_accept_stream_samples(
    message_session: u64,
    active_session: u64,
    stream_session: u64,
) -> bool {
    active_session_matches(active_session, message_session) && message_session == stream_session
}
