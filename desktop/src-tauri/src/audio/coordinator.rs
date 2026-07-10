use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;

use crate::audio::capture::CapturePacket;
use crate::audio::frame::{PreparedFrame, TrackConfigurationRevision};
use crate::audio::preprocess::{downmix_to_mono, rms_level, AudioLevelNormalizer, LinearResampler};
use crate::audio::session::{SessionId, TrackId};
use crate::audio::timeline::{
    ClockMappingRevision, LossAccumulator, Timeline, TimelineEvent, TryDrain,
};

pub const RECORDING_QUEUE_CAPACITY: usize = 128;
pub const LOCAL_ASR_QUEUE_CAPACITY: usize = 64;
pub const EVIDENCE_QUEUE_CAPACITY: usize = 32;
pub const SERVER_TRANSPORT_QUEUE_CAPACITY: usize = 64;
const TARGET_SAMPLE_RATE_HZ: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SinkKind {
    Recording,
    LocalAsr,
    SpeakerEvidence,
    ServerTransport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkOutcome {
    pub kind: SinkKind,
    pub accepted_frames: u64,
    pub dropped_frames: u64,
    pub closed: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkSendError {
    Full,
    Closed,
}

struct BoundedSinkState<T> {
    sender: Mutex<Option<mpsc::SyncSender<T>>>,
    accepted_frames: AtomicU64,
    dropped_frames: AtomicU64,
    queued_frames: AtomicUsize,
    high_water_mark: AtomicUsize,
    closed: AtomicBool,
    close_count: AtomicUsize,
    error: Mutex<Option<String>>,
}

#[derive(Clone)]
pub struct BoundedSink<T> {
    kind: SinkKind,
    state: Arc<BoundedSinkState<T>>,
}

pub struct BoundedReceiver<T> {
    receiver: mpsc::Receiver<T>,
    state: Arc<BoundedSinkState<T>>,
}

pub struct CoordinatorPorts {
    pub recording: BoundedSink<PreparedFrame>,
    pub local_asr: Option<BoundedSink<PreparedFrame>>,
    pub speaker_evidence: Option<BoundedSink<PreparedFrame>>,
    pub server_transport: Option<BoundedSink<PreparedFrame>>,
}

pub fn bounded_sink<T>(kind: SinkKind, capacity: usize) -> (BoundedSink<T>, BoundedReceiver<T>) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let state = Arc::new(BoundedSinkState {
        sender: Mutex::new(Some(sender)),
        accepted_frames: AtomicU64::new(0),
        dropped_frames: AtomicU64::new(0),
        queued_frames: AtomicUsize::new(0),
        high_water_mark: AtomicUsize::new(0),
        closed: AtomicBool::new(false),
        close_count: AtomicUsize::new(0),
        error: Mutex::new(None),
    });
    (
        BoundedSink {
            kind,
            state: Arc::clone(&state),
        },
        BoundedReceiver { receiver, state },
    )
}

impl<T> BoundedSink<T> {
    pub fn try_send(&self, frame: T) -> Result<(), SinkSendError> {
        let sender = match self.state.sender.lock() {
            Ok(sender) => sender,
            Err(_) => {
                self.record_drop("sink state became unavailable");
                return Err(SinkSendError::Closed);
            }
        };
        let Some(sender) = sender.as_ref() else {
            self.record_drop("sink closed");
            return Err(SinkSendError::Closed);
        };
        match sender.try_send(frame) {
            Ok(()) => {
                self.state.accepted_frames.fetch_add(1, Ordering::Relaxed);
                let queued = self.state.queued_frames.fetch_add(1, Ordering::AcqRel) + 1;
                self.observe_high_water_mark(queued);
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.record_drop("sink queue is full");
                Err(SinkSendError::Full)
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.state.closed.store(true, Ordering::Release);
                self.record_drop("sink receiver disconnected");
                Err(SinkSendError::Closed)
            }
        }
    }

    pub fn close(&self) {
        let Ok(mut sender) = self.state.sender.lock() else {
            self.state.closed.store(true, Ordering::Release);
            return;
        };
        if sender.take().is_some() {
            self.state.close_count.fetch_add(1, Ordering::Relaxed);
            self.state.closed.store(true, Ordering::Release);
        }
    }

    pub fn outcome(&self) -> SinkOutcome {
        SinkOutcome {
            kind: self.kind,
            accepted_frames: self.state.accepted_frames.load(Ordering::Acquire),
            dropped_frames: self.state.dropped_frames.load(Ordering::Acquire),
            closed: self.state.closed.load(Ordering::Acquire),
            error: self.state.error.lock().ok().and_then(|error| error.clone()),
        }
    }

    pub fn high_water_mark(&self) -> usize {
        self.state.high_water_mark.load(Ordering::Acquire)
    }

    pub fn close_count(&self) -> usize {
        self.state.close_count.load(Ordering::Acquire)
    }

    fn record_drop(&self, error: &str) {
        self.state.dropped_frames.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut current) = self.state.error.lock() {
            current.get_or_insert_with(|| error.to_string());
        }
    }

    fn observe_high_water_mark(&self, queued: usize) {
        let mut current = self.state.high_water_mark.load(Ordering::Acquire);
        while queued > current {
            match self.state.high_water_mark.compare_exchange_weak(
                current,
                queued,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    #[cfg(debug_assertions)]
                    crate::stt::log_yap(&format!(
                        "audio sink {:?} queue high-water mark={queued}",
                        self.kind
                    ));
                    break;
                }
                Err(observed) => current = observed,
            }
        }
    }
}

impl<T> BoundedReceiver<T> {
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, mpsc::RecvTimeoutError> {
        let item = self.receiver.recv_timeout(timeout)?;
        self.state.queued_frames.fetch_sub(1, Ordering::AcqRel);
        Ok(item)
    }
}

pub struct Coordinator {
    track_id: TrackId,
    ports: CoordinatorPorts,
    timeline: Timeline,
    capture_config: Option<(u16, u32)>,
    track_revision: u32,
    clock_revision: u32,
    last_session_end_ms: u64,
    resampler: Option<LinearResampler>,
    level_normalizer: AudioLevelNormalizer,
    pending_loss: Option<crate::audio::timeline::LossSnapshot>,
    resampler_reset_count: usize,
    revisions_emitted_before_each_resampler_reset: bool,
}

impl Coordinator {
    pub fn new(session_id: SessionId, track_id: TrackId, ports: CoordinatorPorts) -> Self {
        Self {
            timeline: Timeline::new(session_id.clone()),
            track_id,
            ports,
            capture_config: None,
            track_revision: 0,
            clock_revision: 0,
            last_session_end_ms: 0,
            resampler: None,
            level_normalizer: AudioLevelNormalizer::new(),
            pending_loss: None,
            resampler_reset_count: 0,
            revisions_emitted_before_each_resampler_reset: true,
        }
    }

    pub fn consume(
        &mut self,
        packet: &CapturePacket,
        losses: &LossAccumulator,
    ) -> Result<f32, String> {
        self.ensure_configuration(packet)?;
        self.poll_losses(losses)?;

        let channels = usize::from(packet.channels);
        if channels == 0
            || packet.samples.is_empty()
            || !packet.samples.len().is_multiple_of(channels)
        {
            return Err("Invalid captured audio packet.".into());
        }
        let frame_count = u64::try_from(packet.samples.len() / channels)
            .map_err(|_| "Captured audio packet is too large.")?;
        let mut metadata = self
            .timeline
            .frame(
                &self.track_id,
                packet.source_position_frames,
                frame_count,
                packet.channels,
            )
            .map_err(|error| format!("Capture timeline frame failed: {error}"))?;
        self.last_session_end_ms = metadata
            .start_ms
            .checked_add(u64::from(metadata.duration_ms))
            .ok_or_else(|| "Capture timeline frame overflowed.".to_string())?;

        let mono = downmix_to_mono(&packet.samples, channels);
        let level = self.level_normalizer.normalized_level(rms_level(&mono));
        let samples = self
            .resampler
            .as_mut()
            .expect("resampler is initialized with the capture configuration")
            .push(&mono);
        if samples.is_empty() {
            return Ok(level);
        }
        metadata.sample_rate_hz = TARGET_SAMPLE_RATE_HZ;
        metadata.channels = 1;
        metadata.sample_count = samples.len();
        let prepared = PreparedFrame {
            metadata,
            samples: Arc::from(samples),
        };
        let _ = self.ports.recording.try_send(prepared.clone());
        for sink in [
            self.ports.local_asr.as_ref(),
            self.ports.speaker_evidence.as_ref(),
            self.ports.server_transport.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let _ = sink.try_send(prepared.clone());
        }
        Ok(level)
    }

    pub fn poll_losses(&mut self, losses: &LossAccumulator) -> Result<(), String> {
        if self.capture_config.is_none() {
            return Ok(());
        }
        match losses.try_drain() {
            Ok(TryDrain::Pending | TryDrain::Empty) => Ok(()),
            Ok(TryDrain::Snapshot(loss)) => self.consume_loss(loss),
            Err(error) => Err(format!("Capture loss timing failed: {error}")),
        }
    }

    pub fn consume_loss(
        &mut self,
        loss: crate::audio::timeline::LossSnapshot,
    ) -> Result<(), String> {
        if self.capture_config.is_none() {
            self.pending_loss = Some(loss);
            return Ok(());
        }
        self.apply_loss(loss)
    }

    fn apply_loss(&mut self, loss: crate::audio::timeline::LossSnapshot) -> Result<(), String> {
        let gap = self
            .timeline
            .gap(&self.track_id, loss)
            .map_err(|error| format!("Capture timeline gap failed: {error}"))?;
        self.last_session_end_ms = gap
            .start_ms
            .checked_add(u64::from(gap.duration_ms))
            .ok_or_else(|| "Capture timeline gap overflowed.".to_string())?;
        Ok(())
    }

    pub fn close(&mut self) {
        self.ports.recording.close();
        for sink in [
            self.ports.local_asr.as_ref(),
            self.ports.speaker_evidence.as_ref(),
            self.ports.server_transport.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            sink.close();
        }
    }

    pub fn close_sink(&mut self, kind: SinkKind) {
        if let Some(sink) = self.sink(kind) {
            sink.close();
        }
    }

    pub fn outcome(&self, kind: SinkKind) -> Option<SinkOutcome> {
        self.sink(kind).map(BoundedSink::outcome)
    }

    pub fn outcomes(&self) -> Vec<SinkOutcome> {
        [
            SinkKind::Recording,
            SinkKind::LocalAsr,
            SinkKind::SpeakerEvidence,
            SinkKind::ServerTransport,
        ]
        .into_iter()
        .filter_map(|kind| self.outcome(kind))
        .collect()
    }

    pub fn high_water_mark(&self, kind: SinkKind) -> Option<usize> {
        self.sink(kind).map(BoundedSink::high_water_mark)
    }

    pub fn close_count(&self, kind: SinkKind) -> usize {
        self.sink(kind).map_or(0, BoundedSink::close_count)
    }

    pub fn timeline_events(&self) -> &[TimelineEvent] {
        self.timeline.events()
    }

    pub fn resampler_reset_count(&self) -> usize {
        self.resampler_reset_count
    }

    pub fn revisions_emitted_before_each_resampler_reset(&self) -> bool {
        self.revisions_emitted_before_each_resampler_reset
    }

    fn sink(&self, kind: SinkKind) -> Option<&BoundedSink<PreparedFrame>> {
        match kind {
            SinkKind::Recording => Some(&self.ports.recording),
            SinkKind::LocalAsr => self.ports.local_asr.as_ref(),
            SinkKind::SpeakerEvidence => self.ports.speaker_evidence.as_ref(),
            SinkKind::ServerTransport => self.ports.server_transport.as_ref(),
        }
    }

    fn ensure_configuration(&mut self, packet: &CapturePacket) -> Result<(), String> {
        let configuration = (packet.channels, packet.sample_rate_hz);
        if configuration.0 == 0 || configuration.1 == 0 {
            return Err("Invalid microphone configuration.".into());
        }
        if self.capture_config == Some(configuration) {
            return Ok(());
        }
        self.track_revision = self
            .track_revision
            .checked_add(1)
            .ok_or_else(|| "Capture track revision overflowed.".to_string())?;
        self.clock_revision = self
            .clock_revision
            .checked_add(1)
            .ok_or_else(|| "Capture clock revision overflowed.".to_string())?;
        let (source_position_frames, session_time_ms) = if self.capture_config.is_some() {
            (packet.source_position_frames, self.last_session_end_ms)
        } else {
            (0, 0)
        };
        self.timeline
            .configure_track(
                TrackConfigurationRevision::new(
                    self.track_id.clone(),
                    self.track_revision,
                    session_time_ms,
                    packet.sample_rate_hz,
                )
                .map_err(|error| format!("Capture track configuration failed: {error}"))?,
            )
            .map_err(|error| format!("Capture track configuration failed: {error}"))?;
        self.timeline
            .map_clock(
                ClockMappingRevision::new(
                    self.track_id.clone(),
                    self.clock_revision,
                    source_position_frames,
                    session_time_ms,
                )
                .map_err(|error| format!("Capture clock mapping failed: {error}"))?,
            )
            .map_err(|error| format!("Capture clock mapping failed: {error}"))?;
        self.capture_config = Some(configuration);
        self.resampler = Some(LinearResampler::new(
            packet.sample_rate_hz,
            TARGET_SAMPLE_RATE_HZ,
        ));
        self.resampler_reset_count += 1;
        if let Some(loss) = self.pending_loss.take() {
            self.apply_loss(loss)?;
        }
        Ok(())
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::audio::capture::CapturePacket;
    use crate::audio::frame::GapCause;
    use crate::audio::session::{SessionId, TrackId};
    use crate::audio::timeline::{LossAccumulator, TimelineEvent};

    use super::{
        bounded_sink, Coordinator, CoordinatorPorts, SinkKind, EVIDENCE_QUEUE_CAPACITY,
        LOCAL_ASR_QUEUE_CAPACITY, RECORDING_QUEUE_CAPACITY, SERVER_TRANSPORT_QUEUE_CAPACITY,
    };

    fn session() -> SessionId {
        SessionId::new("test-session").unwrap()
    }

    fn track() -> TrackId {
        TrackId::new("test-microphone").unwrap()
    }

    fn packet(position: u64) -> CapturePacket {
        CapturePacket {
            source_position_frames: position,
            channels: 2,
            sample_rate_hz: 48_000,
            samples: [0.25_f32, -0.25].into_iter().cycle().take(960).collect(),
        }
    }

    fn ports(
        recording_capacity: usize,
        local_asr_capacity: Option<usize>,
    ) -> (
        CoordinatorPorts,
        super::BoundedReceiver<crate::audio::frame::PreparedFrame>,
        Option<super::BoundedReceiver<crate::audio::frame::PreparedFrame>>,
    ) {
        let (recording, recording_rx) = bounded_sink(SinkKind::Recording, recording_capacity);
        let (local_asr, local_asr_rx) = local_asr_capacity
            .map(|capacity| bounded_sink(SinkKind::LocalAsr, capacity))
            .map_or((None, None), |(sink, receiver)| {
                (Some(sink), Some(receiver))
            });
        (
            CoordinatorPorts {
                recording,
                local_asr,
                speaker_evidence: None,
                server_transport: None,
            },
            recording_rx,
            local_asr_rx,
        )
    }

    #[test]
    fn recording_continues_when_local_asr_is_absent() {
        let (ports, recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();

        coordinator.consume(&packet(0), &losses).unwrap();

        let frame = recording_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(frame.metadata.sample_rate_hz, 16_000);
        assert_eq!(frame.metadata.channels, 1);
        assert!(coordinator.outcome(SinkKind::LocalAsr).is_none());
    }

    #[test]
    fn stalled_asr_does_not_block_recording_or_callback_intake() {
        let (ports, recording_rx, local_asr_rx) = ports(2, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        let started = Instant::now();

        coordinator.consume(&packet(0), &losses).unwrap();
        coordinator.consume(&packet(480), &losses).unwrap();

        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(
            recording_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            0
        );
        assert_eq!(
            recording_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            1
        );
        assert_eq!(
            local_asr_rx
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            0
        );
        assert_eq!(
            coordinator
                .outcome(SinkKind::LocalAsr)
                .unwrap()
                .dropped_frames,
            1
        );
    }

    #[test]
    fn one_sink_failure_does_not_close_other_sinks() {
        let (ports, recording_rx, local_asr_rx) = ports(1, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        coordinator.close_sink(SinkKind::LocalAsr);
        drop(local_asr_rx);

        coordinator.consume(&packet(0), &losses).unwrap();

        assert_eq!(
            recording_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .metadata
                .sequence,
            0
        );
        assert!(!coordinator.outcome(SinkKind::Recording).unwrap().closed);
        assert!(coordinator.outcome(SinkKind::LocalAsr).unwrap().closed);
    }

    #[test]
    fn finalization_closes_every_sink_exactly_once() {
        let (ports, _, _) = ports(1, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);

        coordinator.close();
        coordinator.close();

        for outcome in coordinator.outcomes() {
            assert!(outcome.closed);
            assert_eq!(coordinator.close_count(outcome.kind), 1);
        }
    }

    #[test]
    fn composed_result_marks_only_the_failed_or_degraded_sinks() {
        let (ports, recording_rx, local_asr_rx) = ports(1, Some(1));
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        coordinator.close_sink(SinkKind::LocalAsr);
        drop(local_asr_rx);

        coordinator.consume(&packet(0), &losses).unwrap();
        drop(recording_rx);

        let recording = coordinator.outcome(SinkKind::Recording).unwrap();
        let asr = coordinator.outcome(SinkKind::LocalAsr).unwrap();
        assert_eq!(recording.dropped_frames, 0);
        assert_eq!(recording.error, None);
        assert!(asr.closed);
        assert!(asr.error.is_some());
    }

    #[test]
    fn queue_capacities_and_high_water_marks_are_visible() {
        assert_eq!(RECORDING_QUEUE_CAPACITY, 128);
        assert_eq!(LOCAL_ASR_QUEUE_CAPACITY, 64);
        assert_eq!(EVIDENCE_QUEUE_CAPACITY, 32);
        assert_eq!(SERVER_TRANSPORT_QUEUE_CAPACITY, 64);

        let (ports, _recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();
        coordinator.consume(&packet(0), &losses).unwrap();
        coordinator.consume(&packet(480), &losses).unwrap();

        assert_eq!(coordinator.high_water_mark(SinkKind::Recording), Some(2));
    }

    #[test]
    fn source_positions_and_losses_leave_a_timeline_gap() {
        let (ports, recording_rx, _) = ports(1, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = Arc::new(LossAccumulator::new());
        losses.record(0, 480, GapCause::SinkUnavailable);

        coordinator.consume(&packet(480), &losses).unwrap();

        let frame = recording_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(frame.metadata.start_ms, 10);
        assert!(coordinator.timeline_events().iter().any(|event| matches!(event, TimelineEvent::Gap(gap) if gap.start_ms == 0 && gap.duration_ms == 10)));
    }

    #[test]
    fn resampler_resets_only_after_track_and_clock_revisions() {
        let (ports, _recording_rx, _) = ports(2, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let losses = LossAccumulator::new();

        coordinator.consume(&packet(0), &losses).unwrap();
        coordinator
            .consume(
                &CapturePacket {
                    sample_rate_hz: 44_100,
                    ..packet(480)
                },
                &losses,
            )
            .unwrap();

        assert_eq!(coordinator.resampler_reset_count(), 2);
        assert!(coordinator.revisions_emitted_before_each_resampler_reset());
    }

    #[test]
    fn sink_workers_shutdown_after_ports_close() {
        let (ports, recording_rx, _) = ports(1, None);
        let mut coordinator = Coordinator::new(session(), track(), ports);
        let worker = std::thread::spawn(move || {
            while recording_rx.recv_timeout(Duration::from_millis(10)).is_ok() {}
        });

        coordinator.close();

        worker.join().unwrap();
    }
}
