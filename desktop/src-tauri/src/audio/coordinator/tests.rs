use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

use crate::audio::capture::{new_callback_boundary, CapturePacket};
use crate::audio::frame::{GapCause, PreparedFrame};
use crate::audio::recording::{CaptureStatus, RecordingSinkHandle};
use crate::audio::session::{SessionId, TrackId};
use crate::audio::timeline::{LossAccumulator, LossSnapshot, RecordingInput};

use super::{
    bounded_sink, Coordinator, CoordinatorPorts, RevisionEvent, SinkKind, SinkSendError,
    EVIDENCE_QUEUE_CAPACITY, LOCAL_ASR_QUEUE_CAPACITY, RECORDING_QUEUE_CAPACITY,
    SERVER_TRANSPORT_QUEUE_CAPACITY,
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
    super::BoundedReceiver<RecordingInput>,
    Option<super::BoundedReceiver<PreparedFrame>>,
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

fn recv_recording_frame(receiver: &super::BoundedReceiver<RecordingInput>) -> PreparedFrame {
    loop {
        match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
            RecordingInput::PreparedFrame(frame) => return frame,
            RecordingInput::RevisionTransition(_) | RecordingInput::Gap(_) => {}
        }
    }
}

fn recv_recording_gap(
    receiver: &super::BoundedReceiver<RecordingInput>,
) -> crate::audio::frame::AudioGap {
    loop {
        match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
            RecordingInput::Gap(gap) => return gap,
            RecordingInput::RevisionTransition(_) | RecordingInput::PreparedFrame(_) => {}
        }
    }
}

fn recv_recording_inputs(
    receiver: &super::BoundedReceiver<RecordingInput>,
    count: usize,
) -> Vec<RecordingInput> {
    (0..count)
        .map(|_| receiver.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect()
}

fn persistent_coordinator(label: &str) -> (std::path::PathBuf, Coordinator, RecordingSinkHandle) {
    let directory = std::env::temp_dir().join(format!("yap-{label}-{}", std::process::id()));
    std::fs::remove_dir_all(&directory).ok();
    std::fs::create_dir_all(&directory).unwrap();
    let session = SessionId::new(label).unwrap();
    let (ports, recording_rx, _) = ports(RECORDING_QUEUE_CAPACITY, None);
    let recording = RecordingSinkHandle::spawn(
        directory.clone(),
        session.clone(),
        ports.recording.clone(),
        recording_rx,
    );
    (
        directory,
        Coordinator::new(session, track(), ports),
        recording,
    )
}

fn assert_rejection_is_recording_terminal(
    directory: std::path::PathBuf,
    mut coordinator: Coordinator,
    recording: RecordingSinkHandle,
    rejection: String,
    expected: &str,
) {
    let degradation = coordinator.outcome(SinkKind::Recording).unwrap().error;
    coordinator.close();
    let result = recording.finalize().unwrap();
    std::fs::remove_dir_all(directory).unwrap();

    assert_eq!(rejection, expected);
    assert_eq!(degradation.as_deref(), Some(expected));
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
}

fn retained_timeline_metadata(coordinator: &Coordinator) -> usize {
    coordinator.timeline.retained_metadata_count()
}

mod lifecycle;
mod queue_semantics;
mod timeline_failures;
