use super::*;
use crate::audio::capture::{new_callback_boundary, CapturePacket, CapturePorts};
use crate::audio::coordinator::{
    bounded_sink, BoundedReceiver, BoundedSink, Coordinator, CoordinatorPorts, SinkKind,
};
use crate::audio::frame::{AudioFrame, GapCause, PreparedFrame};
use crate::audio::recording::{
    allocate_recording_session, scan_recordings, CaptureStatus, RecordingSinkHandle,
};
use crate::audio::session::{SessionId, TrackId};
use crate::audio::timeline::{LossAccumulator, LossSnapshot, RecordingInput};
use std::sync::Barrier;

fn capture_loss_coordinator() -> (
    Arc<Mutex<Coordinator>>,
    BoundedSink<RecordingInput>,
    BoundedReceiver<RecordingInput>,
) {
    let (recording, receiver) = bounded_sink(SinkKind::Recording, 8);
    let coordinator = Coordinator::new(
        SessionId::new("runtime-loss-test").unwrap(),
        TrackId::new("live-microphone").unwrap(),
        CoordinatorPorts {
            recording: recording.clone(),
            local_asr: None,
            speaker_evidence: None,
            server_transport: None,
        },
    );
    (Arc::new(Mutex::new(coordinator)), recording, receiver)
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

mod adapters;
mod capture;
mod lifecycle;
mod telemetry;
mod warmup_finalization;
