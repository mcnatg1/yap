use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use yap_desktop_lib::audio::capture::CapturePacket;
use yap_desktop_lib::audio::coordinator::{
    bounded_sink, Coordinator, CoordinatorPorts, SinkKind, RECORDING_QUEUE_CAPACITY,
};
use yap_desktop_lib::audio::frame::GapCause;
use yap_desktop_lib::audio::recording::{scan_recordings, CaptureStatus, RecordingSinkHandle};
use yap_desktop_lib::audio::session::{SessionId, TrackId};
use yap_desktop_lib::audio::timeline::{LossSnapshot, RecordingInput};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn temp_directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "yap-audio-foundation-{name}-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn session_id(name: &str) -> SessionId {
    SessionId::new(format!("audio-foundation-{name}")).unwrap()
}

fn packet(source_position_frames: u64, samples: Vec<f32>) -> CapturePacket {
    CapturePacket {
        source_position_frames,
        channels: 1,
        sample_rate_hz: 16_000,
        samples,
    }
}

fn recording_coordinator(
    directory: PathBuf,
    session_id: SessionId,
) -> (Coordinator, RecordingSinkHandle) {
    let (recording, receiver) =
        bounded_sink::<RecordingInput>(SinkKind::Recording, RECORDING_QUEUE_CAPACITY);
    let handle =
        RecordingSinkHandle::spawn(directory, session_id.clone(), recording.clone(), receiver);
    let coordinator = Coordinator::new(
        session_id,
        TrackId::new("live-microphone").unwrap(),
        CoordinatorPorts {
            recording,
            local_asr: None,
            speaker_evidence: None,
            server_transport: None,
        },
    );
    (coordinator, handle)
}

fn sidecar_json(directory: &std::path::Path, session_id: &SessionId) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(directory.join(format!("live-{session_id}.capture.json"))).unwrap(),
    )
    .unwrap()
}

fn fixture_tone_wav() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const SAMPLE_COUNT: usize = 4_000;
    let mut wav = Vec::with_capacity(44 + SAMPLE_COUNT * 2);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + SAMPLE_COUNT as u32 * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(SAMPLE_COUNT as u32 * 2).to_le_bytes());
    for index in 0..SAMPLE_COUNT {
        let seconds = index as f64 / f64::from(SAMPLE_RATE);
        let envelope = (index as f64 / 120.0)
            .min((SAMPLE_COUNT - index) as f64 / 120.0)
            .clamp(0.0, 1.0);
        let value = (2.0 * std::f64::consts::PI * 440.0 * seconds).sin() * 0.25 * envelope;
        wav.extend_from_slice(&((value * f64::from(i16::MAX)).round() as i16).to_le_bytes());
    }
    wav
}

fn fixture_tone_samples() -> Vec<f32> {
    fixture_tone_wav()[44..]
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes(sample.try_into().unwrap()) as f32 / i16::MAX as f32)
        .collect()
}

#[test]
fn callback_loss_is_committed_with_exact_source_aware_timeline_fields() {
    let directory = temp_directory("exact-loss");
    let session_id = session_id("exact-loss");
    let (mut coordinator, recording) = recording_coordinator(directory.clone(), session_id.clone());

    coordinator
        .consume(&packet(0, vec![0.0; 4_000]), &Default::default())
        .unwrap();
    coordinator
        .consume_loss(LossSnapshot {
            first_source_position_frames: 4_000,
            dropped_frames: 1_600,
            cause: GapCause::DeviceDiscontinuity,
            generation: 1,
        })
        .unwrap();
    coordinator
        .consume(&packet(5_600, vec![0.0; 400]), &Default::default())
        .unwrap();
    coordinator.close();

    let result = recording.finalize().unwrap();
    assert_eq!(result.status, CaptureStatus::Complete);

    let sidecar = sidecar_json(&directory, &session_id);
    let gap = &sidecar["timelineGaps"][0];
    assert_eq!(gap["sessionId"], session_id.as_str());
    assert_eq!(gap["trackId"], "live-microphone");
    assert_eq!(gap["startMs"], 250);
    assert_eq!(gap["durationMs"], 100);
    assert_eq!(gap["sourcePositionFrames"], 4_000);
    assert_eq!(gap["droppedFrames"], 1_600);
    assert_eq!(gap["cause"], "device_discontinuity");
    assert_eq!(gap["generation"], 1);
    assert_eq!(sidecar["trackConfigurations"][0]["revision"], 1);
    assert_eq!(sidecar["clockMappings"][0]["sourcePositionFrames"], 0);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn closed_recording_sink_cannot_publish_a_complete_capture() {
    let directory = temp_directory("closed-sink");
    let session_id = session_id("closed-sink");
    let (mut coordinator, recording) = recording_coordinator(directory.clone(), session_id.clone());

    coordinator.close_sink(SinkKind::Recording);
    coordinator
        .consume(&packet(0, vec![0.0; 400]), &Default::default())
        .unwrap();

    let result = recording.finalize().unwrap();
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(scan_recordings(&directory).unwrap().complete.is_empty());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn preconfiguration_loss_followed_by_close_cannot_publish_complete() {
    let directory = temp_directory("preconfiguration-loss-close");
    let session_id = session_id("preconfiguration-loss-close");
    let (mut coordinator, recording) = recording_coordinator(directory.clone(), session_id);

    coordinator
        .consume_loss(LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 1_600,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        })
        .unwrap();
    coordinator.close();

    let result = recording.finalize().unwrap();
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    let scan = scan_recordings(&directory).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn saturated_recording_sink_cannot_publish_a_complete_capture() {
    let directory = temp_directory("saturated-sink");
    let session_id = session_id("saturated-sink");
    let (recording_sink, receiver) = bounded_sink::<RecordingInput>(SinkKind::Recording, 1);
    let mut coordinator = Coordinator::new(
        session_id.clone(),
        TrackId::new("live-microphone").unwrap(),
        CoordinatorPorts {
            recording: recording_sink.clone(),
            local_asr: None,
            speaker_evidence: None,
            server_transport: None,
        },
    );

    coordinator
        .consume(&packet(0, vec![0.0; 400]), &Default::default())
        .unwrap();
    assert!(
        coordinator
            .outcome(SinkKind::Recording)
            .unwrap()
            .dropped_frames
            > 0
    );
    coordinator.close();
    let recording =
        RecordingSinkHandle::spawn(directory.clone(), session_id, recording_sink, receiver);

    let result = recording.finalize().unwrap();
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert!(scan_recordings(&directory).unwrap().complete.is_empty());

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn four_hour_timeline_churn_is_bounded_monotonic_and_retains_only_written_pcm() {
    const FOUR_HOURS_FRAMES: u64 = 4 * 60 * 60 * 16_000;
    const LOSS_EVENTS: u64 = 1_090;
    const COALESCED_PREFIX: u64 = 64;

    let directory = temp_directory("four-hours");
    let session_id = session_id("four-hours");
    let (mut coordinator, recording) = recording_coordinator(directory.clone(), session_id.clone());

    coordinator
        .consume(&packet(0, vec![0.0; 16]), &Default::default())
        .unwrap();
    let frames_per_loss = FOUR_HOURS_FRAMES / LOSS_EVENTS;
    let mut source_position = 16_u64;
    for index in 0..LOSS_EVENTS {
        let dropped_frames = if index + 1 == LOSS_EVENTS {
            16 + FOUR_HOURS_FRAMES - source_position
        } else {
            frames_per_loss
        };
        let cause = if index < COALESCED_PREFIX {
            GapCause::CallbackPoolExhausted
        } else if index.is_multiple_of(2) {
            GapCause::DeviceDiscontinuity
        } else {
            GapCause::SinkUnavailable
        };
        coordinator
            .consume_loss(LossSnapshot {
                first_source_position_frames: source_position,
                dropped_frames,
                cause,
                generation: index + 1,
            })
            .unwrap();
        source_position += dropped_frames;
        if index % 16 == 15 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    assert_eq!(source_position, 16 + FOUR_HOURS_FRAMES);
    let outcome = coordinator.outcome(SinkKind::Recording).unwrap();
    assert_eq!(outcome.dropped_frames, 0);
    assert_eq!(outcome.accepted_frames, LOSS_EVENTS + 2);
    assert!(coordinator.high_water_mark(SinkKind::Recording).unwrap() <= RECORDING_QUEUE_CAPACITY);
    coordinator.close();

    let result = recording.finalize().unwrap();
    assert_eq!(result.status, CaptureStatus::Partial);
    assert!(result.committed.is_none());
    assert_eq!(
        fs::metadata(directory.join(format!("live-{session_id}.wav.part")))
            .unwrap()
            .len(),
        76
    );
    assert!(
        fs::metadata(directory.join(format!("live-{session_id}.capture.journal.part")))
            .unwrap()
            .len()
            <= 512 * 1024
    );
    let scan = scan_recordings(&directory).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.partial.len(), 1);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn generated_tone_commits_a_valid_wav_and_scans_as_complete() {
    let directory = temp_directory("tone");
    let session_id = session_id("tone");
    let (mut coordinator, recording) = recording_coordinator(directory.clone(), session_id.clone());
    let tone = fixture_tone_samples();

    coordinator
        .consume(&packet(0, tone), &Default::default())
        .unwrap();
    coordinator.close();
    let result = recording.finalize().unwrap();
    let committed = result.committed.unwrap();
    let wav = fs::read(directory.join(&committed.manifest.audio_file)).unwrap();

    assert_eq!(result.status, CaptureStatus::Complete);
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 8_000);
    let hash = Sha256::digest(&wav)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(hash, committed.manifest.audio_sha256);
    assert_eq!(wav, fixture_tone_wav());
    assert_eq!(scan_recordings(&directory).unwrap().complete.len(), 1);

    fs::remove_dir_all(directory).unwrap();
}
