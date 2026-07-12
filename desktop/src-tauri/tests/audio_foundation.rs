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

#[derive(Default)]
struct DurableJournalTruth {
    session_id: Option<String>,
    track_configurations: Vec<serde_json::Value>,
    clock_mappings: Vec<serde_json::Value>,
    timeline_gaps: Vec<serde_json::Value>,
    sequence_coverage: Vec<serde_json::Value>,
    sink_degraded: bool,
    overflow_reason: Option<String>,
}

fn replay_journal(directory: &std::path::Path, session_id: &SessionId) -> DurableJournalTruth {
    let journal_path = directory.join(format!("live-{session_id}.capture.journal.part"));
    let text = fs::read_to_string(journal_path).unwrap();
    let mut truth = DurableJournalTruth::default();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        match record["kind"].as_str().unwrap() {
            "header" => {
                let journal = &record["journal"];
                truth.session_id = journal["sessionId"].as_str().map(str::to_string);
                truth.track_configurations =
                    journal["trackConfigurations"].as_array().unwrap().clone();
                truth.clock_mappings = journal["clockMappings"].as_array().unwrap().clone();
                truth.timeline_gaps = journal["timelineGaps"].as_array().unwrap().clone();
                truth.sequence_coverage = journal["sequenceCoverage"].as_array().unwrap().clone();
                truth.sink_degraded = journal["sinkDegraded"].as_bool().unwrap();
            }
            "delta" => {
                let delta = &record["delta"];
                assert_eq!(delta["sessionId"], session_id.as_str());
                for transition in delta["revisionTransitions"].as_array().unwrap() {
                    truth
                        .track_configurations
                        .push(transition["configuration"].clone());
                    truth
                        .clock_mappings
                        .push(transition["clockMapping"].clone());
                }
                let gap_start = delta["timelineGapStartIndex"].as_u64().unwrap() as usize;
                truth.timeline_gaps.truncate(gap_start);
                truth
                    .timeline_gaps
                    .extend(delta["timelineGaps"].as_array().unwrap().iter().cloned());
                for coverage in delta["sequenceCoverage"].as_array().unwrap() {
                    let track_id = coverage["trackId"].as_str().unwrap();
                    if let Some(existing) = truth
                        .sequence_coverage
                        .iter_mut()
                        .find(|existing| existing["trackId"] == track_id)
                    {
                        *existing = coverage.clone();
                    } else {
                        truth.sequence_coverage.push(coverage.clone());
                    }
                }
                truth.sink_degraded |= delta["sinkDegraded"].as_bool().unwrap();
            }
            "overflow" => {
                assert_eq!(record["session_id"], session_id.as_str());
                truth.overflow_reason = record["reason"].as_str().map(str::to_string);
            }
            kind => panic!("unexpected recording journal record: {kind}"),
        }
    }
    truth
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
        result.error.as_deref(),
        Some("recording journal durability stopped: journal size limit reached")
    );
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
    let truth = replay_journal(&directory, &session_id);
    assert_eq!(truth.session_id.as_deref(), Some(session_id.as_str()));
    assert_eq!(truth.track_configurations.len(), 1);
    assert_eq!(truth.track_configurations[0]["trackId"], "live-microphone");
    assert_eq!(truth.track_configurations[0]["revision"], 1);
    assert_eq!(truth.track_configurations[0]["effectiveAtMs"], 0);
    assert_eq!(truth.track_configurations[0]["sampleRateHz"], 16_000);
    assert_eq!(truth.clock_mappings.len(), 1);
    assert_eq!(truth.clock_mappings[0]["trackId"], "live-microphone");
    assert_eq!(truth.clock_mappings[0]["revision"], 1);
    assert_eq!(truth.clock_mappings[0]["sourcePositionFrames"], 0);
    assert_eq!(truth.clock_mappings[0]["sessionTimeMs"], 0);
    assert_eq!(truth.sequence_coverage.len(), 1);
    assert_eq!(truth.sequence_coverage[0]["firstSequence"], 0);
    assert_eq!(truth.sequence_coverage[0]["lastSequence"], 0);
    assert!(!truth.sink_degraded);
    assert_eq!(
        truth.overflow_reason.as_deref(),
        Some("journal size limit reached")
    );

    assert_eq!(truth.timeline_gaps.len(), 1_014);
    let frames_per_loss_ms = frames_per_loss * 1_000 / 16_000;
    let prefix = &truth.timeline_gaps[0];
    assert_eq!(prefix["sessionId"], session_id.as_str());
    assert_eq!(prefix["trackId"], "live-microphone");
    assert_eq!(prefix["startMs"], 1);
    assert_eq!(prefix["durationMs"], frames_per_loss_ms * COALESCED_PREFIX);
    assert_eq!(prefix["sourcePositionFrames"], 16);
    assert_eq!(prefix["droppedFrames"], frames_per_loss * COALESCED_PREFIX);
    assert_eq!(prefix["cause"], "callback_pool_exhausted");
    assert_eq!(prefix["generation"], COALESCED_PREFIX);

    for (gap_index, gap) in truth.timeline_gaps.iter().enumerate().skip(1) {
        let loss_index = COALESCED_PREFIX + gap_index as u64 - 1;
        let expected_source = 16 + loss_index * frames_per_loss;
        let expected_start_ms = 1 + loss_index * frames_per_loss_ms;
        let expected_cause = if loss_index.is_multiple_of(2) {
            "device_discontinuity"
        } else {
            "sink_unavailable"
        };
        assert_eq!(gap["sessionId"], session_id.as_str());
        assert_eq!(gap["trackId"], "live-microphone");
        assert_eq!(gap["startMs"], expected_start_ms);
        assert_eq!(gap["durationMs"], frames_per_loss_ms);
        assert_eq!(gap["sourcePositionFrames"], expected_source);
        assert_eq!(gap["droppedFrames"], frames_per_loss);
        assert_eq!(gap["cause"], expected_cause);
        assert_eq!(gap["generation"], loss_index + 1);
    }

    let lineage = result.partial_lineage.as_ref().unwrap();
    let partial_sidecar_path = directory.join(&lineage.capture_sidecar_file);
    let partial_sidecar = fs::read(&partial_sidecar_path).unwrap();
    let partial_hash = Sha256::digest(&partial_sidecar)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(partial_hash, lineage.capture_sidecar_sha256);
    let partial_sidecar: serde_json::Value = serde_json::from_slice(&partial_sidecar).unwrap();
    assert_eq!(partial_sidecar["sessionId"], session_id.as_str());
    assert_eq!(partial_sidecar["status"], "partial");
    assert!(!directory
        .join(format!("live-{session_id}.capture.json"))
        .exists());
    assert!(!directory
        .join(format!("live-{session_id}.commit.json"))
        .exists());
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
