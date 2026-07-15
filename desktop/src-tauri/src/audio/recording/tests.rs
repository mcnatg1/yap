
use super::*;
use crate::audio::coordinator::{bounded_sink, SinkKind};
use crate::audio::frame::AudioFrame;
use crate::audio::session::TrackId;
use crate::audio::session::{SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode};
use std::sync::Arc;

mod commit_partial;
mod completion_races;
mod journal_durability;
mod limits_handles;
mod publication_identity;
mod publication_security;
mod recovery_scan;
mod reservation_publication;
mod scanner_validation;
mod worker_failures;

#[cfg(unix)]
fn create_file_symlink_for_test(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn create_file_symlink_for_test(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

fn assert_hash_bound_sidecar_mutation_is_damaged<F>(label: &str, mutate: F)
where
    F: FnOnce(&mut serde_json::Value),
{
    let dir = tempfile_dir(label);
    let session = SessionId::new(format!("s-{label}")).unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    recording
        .append_input(RecordingInput::PreparedFrame(prepared_frame(&session)))
        .unwrap();
    assert_eq!(
        recording.finalize().unwrap().status,
        CaptureStatus::Complete
    );

    let sidecar_path = dir.join(format!("live-{session}.capture.json"));
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    mutate(&mut sidecar);
    fs::write(&sidecar_path, serde_json::to_vec(&sidecar).unwrap()).unwrap();
    rehash_capture_sidecar(&dir, &session, &sidecar_path);

    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty(), "{label}");
    assert_eq!(scan.damaged.len(), 1, "{label}");
    fs::remove_dir_all(dir).ok();
}

fn assert_hash_bound_gap_mutation_is_damaged<F>(label: &str, mutate: F)
where
    F: FnOnce(&mut serde_json::Value),
{
    let dir = tempfile_dir(label);
    let session = SessionId::new(format!("s-{label}")).unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    recording
        .append_input(recording_revision(&track, 2, 10, 8_000, 160))
        .unwrap();
    recording
        .append_input(RecordingInput::Gap(AudioGap {
            session_id: session.clone(),
            track_id: track,
            start_ms: 10,
            duration_ms: 10,
            source_position_frames: 160,
            dropped_frames: 80,
            cause: GapCause::DeviceDiscontinuity,
            generation: 1,
        }))
        .unwrap();
    let mut frame = prepared_frame_at(&session, 0, 20);
    frame.metadata.sample_rate_hz = 8_000;
    recording
        .append_input(RecordingInput::PreparedFrame(frame))
        .unwrap();
    assert_eq!(
        recording.finalize().unwrap().status,
        CaptureStatus::Complete
    );

    let sidecar_path = dir.join(format!("live-{session}.capture.json"));
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    mutate(&mut sidecar);
    fs::write(&sidecar_path, serde_json::to_vec(&sidecar).unwrap()).unwrap();
    rehash_capture_sidecar(&dir, &session, &sidecar_path);

    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty(), "{label}");
    assert_eq!(scan.damaged.len(), 1, "{label}");
    fs::remove_dir_all(dir).ok();
}

fn revision_transition(
    track: &TrackId,
    revision: u32,
    effective_at_ms: u64,
    sample_rate_hz: u32,
    source_position_frames: u64,
) -> RecordingRevisionTransition {
    RecordingRevisionTransition::new(
        TrackConfigurationRevision::new(track.clone(), revision, effective_at_ms, sample_rate_hz)
            .unwrap(),
        ClockMappingRevision::new(
            track.clone(),
            revision,
            source_position_frames,
            effective_at_ms,
        )
        .unwrap(),
    )
    .unwrap()
}

fn recording_revision(
    track: &TrackId,
    revision: u32,
    effective_at_ms: u64,
    sample_rate_hz: u32,
    source_position_frames: u64,
) -> RecordingInput {
    RecordingInput::RevisionTransition(revision_transition(
        track,
        revision,
        effective_at_ms,
        sample_rate_hz,
        source_position_frames,
    ))
}

fn prepared_frame(session_id: &SessionId) -> PreparedFrame {
    prepared_frame_at(session_id, 0, 0)
}

fn prepared_frame_at(session_id: &SessionId, sequence: u64, start_ms: u64) -> PreparedFrame {
    PreparedFrame {
        metadata: AudioFrame {
            session_id: session_id.clone(),
            track_id: TrackId::new("live-microphone").unwrap(),
            sequence,
            sample_rate_hz: 16_000,
            channels: 1,
            start_ms,
            duration_ms: 1,
            sample_count: 1,
        },
        samples: Arc::from([0.25]),
    }
}

fn rehash_capture_sidecar(dir: &Path, session: &SessionId, sidecar_path: &Path) {
    let commit_path = dir.join(format!("live-{session}.commit.json"));
    let mut commit: CaptureCommitManifest =
        serde_json::from_slice(&fs::read(&commit_path).unwrap()).unwrap();
    commit.capture_sidecar_sha256 = sha256_file(sidecar_path).unwrap();
    fs::write(commit_path, serde_json::to_vec(&commit).unwrap()).unwrap();
}

fn tempfile_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yap-recording-{label}-{}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn recover_partial_wav_for_test(
    directory: &Path,
    session_id: &SessionId,
) -> Result<(String, u64, String), String> {
    let partial = directory.join(format!("live-{session_id}.wav.part"));
    let final_wav = directory.join(format!("live-{session_id}.wav"));
    let path = if partial.is_file() {
        partial
    } else {
        final_wav
    };
    let admitted = admit_expected_private_regular_artifact(&path, &path)?;
    recover_partial_wav_with_identity(directory, session_id, &admitted)
}
