use super::*;

#[test]
fn scanner_reports_audio_hash_and_sidecar_damage_separately() {
    for (label, corrupt) in [
        ("damaged-audio-hash", "audio"),
        ("damaged-sidecar", "sidecar"),
    ] {
        let dir = tempfile_dir(label);
        let session = SessionId::new(format!("s-{label}")).unwrap();
        let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
        recording.append_pcm16(&[1, 0]).unwrap();
        recording.finalize().unwrap();
        let name = match corrupt {
            "audio" => format!("live-{session}.wav"),
            _ => format!("live-{session}.capture.json"),
        };
        fs::write(dir.join(name), b"corrupted").unwrap();

        let scan = scan_recordings(&dir).unwrap();

        assert!(scan.complete.is_empty());
        assert!(scan.partial.is_empty());
        assert_eq!(scan.damaged.len(), 1, "{label}");
        assert_eq!(scan.damaged[0].session_id, session, "{label}");
        assert!(
            scan.damaged[0].reason.contains("Damaged complete"),
            "{label}"
        );
        fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn scanner_rejects_hash_bound_invalid_timeline_metadata() {
    let dir = tempfile_dir("invalid-timeline-sidecar");
    let session = SessionId::new("s-invalid-timeline-sidecar").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    recording
        .append_input(RecordingInput::PreparedFrame(prepared_frame(&session)))
        .unwrap();
    recording
        .append_input(RecordingInput::Gap(AudioGap {
            session_id: session.clone(),
            track_id: track,
            start_ms: 10,
            duration_ms: 10,
            source_position_frames: 160,
            dropped_frames: 160,
            cause: crate::audio::frame::GapCause::DeviceDiscontinuity,
            generation: 1,
        }))
        .unwrap();
    recording.finalize().unwrap();

    let sidecar_path = dir.join(format!("live-{session}.capture.json"));
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    sidecar["timelineGaps"][0]["durationMs"] = serde_json::Value::from(0);
    fs::write(&sidecar_path, serde_json::to_vec(&sidecar).unwrap()).unwrap();

    let commit_path = dir.join(format!("live-{session}.commit.json"));
    let mut commit: CaptureCommitManifest =
        serde_json::from_slice(&fs::read(&commit_path).unwrap()).unwrap();
    commit.capture_sidecar_sha256 = sha256_file(&sidecar_path).unwrap();
    fs::write(&commit_path, serde_json::to_vec(&commit).unwrap()).unwrap();

    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.damaged.len(), 1);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn scanner_rejects_a_recorded_track_without_coordinator_revisions() {
    let dir = tempfile_dir("missing-track-revisions");
    let session = SessionId::new("s-missing-track-revisions").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    recording
        .append_input(RecordingInput::PreparedFrame(prepared_frame(&session)))
        .unwrap();
    recording.finalize().unwrap();

    let sidecar_path = dir.join(format!("live-{session}.capture.json"));
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    sidecar["trackConfigurations"] = serde_json::json!([]);
    sidecar["clockMappings"] = serde_json::json!([]);
    fs::write(&sidecar_path, serde_json::to_vec(&sidecar).unwrap()).unwrap();
    rehash_capture_sidecar(&dir, &session, &sidecar_path);

    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.damaged.len(), 1);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn scanner_rejects_a_configuration_without_its_latest_clock_mapping() {
    let dir = tempfile_dir("missing-latest-clock-mapping");
    let session = SessionId::new("s-missing-latest-clock-mapping").unwrap();
    let track = TrackId::new("live-microphone").unwrap();
    let mut recording = StreamingRecording::create(&dir, session.clone()).unwrap();
    recording
        .append_input(recording_revision(&track, 1, 0, 16_000, 0))
        .unwrap();
    recording
        .append_input(RecordingInput::PreparedFrame(prepared_frame(&session)))
        .unwrap();
    recording.finalize().unwrap();

    let sidecar_path = dir.join(format!("live-{session}.capture.json"));
    let mut sidecar: serde_json::Value =
        serde_json::from_slice(&fs::read(&sidecar_path).unwrap()).unwrap();
    sidecar["trackConfigurations"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "trackId": "live-microphone",
            "revision": 2,
            "effectiveAtMs": 1,
            "sampleRateHz": 16_000
        }));
    fs::write(&sidecar_path, serde_json::to_vec(&sidecar).unwrap()).unwrap();
    rehash_capture_sidecar(&dir, &session, &sidecar_path);

    let scan = scan_recordings(&dir).unwrap();
    assert!(scan.complete.is_empty());
    assert_eq!(scan.damaged.len(), 1);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn scanner_rejects_hash_bound_sink_degradation() {
    assert_hash_bound_sidecar_mutation_is_damaged("sink-degraded", |sidecar| {
        sidecar["sinkDegraded"] = serde_json::Value::Bool(true);
    });
}

#[test]
fn scanner_rejects_hash_bound_sequence_gap_metadata() {
    assert_hash_bound_sidecar_mutation_is_damaged("sequence-gap", |sidecar| {
        sidecar["sequenceGaps"] = serde_json::json!([{
            "trackId": "live-microphone",
            "firstSequence": 1,
            "droppedFrames": 1
        }]);
    });
}

#[test]
fn scanner_rejects_hash_bound_sequence_gap_overflow() {
    assert_hash_bound_sidecar_mutation_is_damaged("sequence-gap-overflow", |sidecar| {
        sidecar["sequenceGapOverflow"] = serde_json::json!({
            "detailCapacity": 1_024,
            "omittedGapCount": 1,
            "omittedDroppedFrames": 1
        });
    });
}

#[test]
fn scanner_rejects_hash_bound_malformed_sequence_coverage() {
    assert_hash_bound_sidecar_mutation_is_damaged("sequence-coverage", |sidecar| {
        sidecar["sequenceCoverage"][0]["firstSequence"] = serde_json::Value::from(2);
        sidecar["sequenceCoverage"][0]["lastSequence"] = serde_json::Value::from(1);
    });
}

#[test]
fn scanner_rejects_hash_bound_sequence_coverage_that_starts_after_zero() {
    assert_hash_bound_sidecar_mutation_is_damaged("sequence-prefix", |sidecar| {
        sidecar["sequenceCoverage"][0]["firstSequence"] = serde_json::Value::from(5);
        sidecar["sequenceCoverage"][0]["lastSequence"] = serde_json::Value::from(5);
    });
}

#[test]
fn scanner_rejects_hash_bound_mismatched_revision_transition_timestamp() {
    assert_hash_bound_sidecar_mutation_is_damaged("revision-timestamp", |sidecar| {
        sidecar["clockMappings"][0]["sessionTimeMs"] = serde_json::Value::from(1);
    });
}

#[test]
fn scanner_rejects_hash_bound_gap_with_wrong_duration() {
    assert_hash_bound_gap_mutation_is_damaged("gap-wrong-duration", |sidecar| {
        sidecar["timelineGaps"][0]["durationMs"] = serde_json::Value::from(11);
    });
}

#[test]
fn scanner_rejects_hash_bound_gap_with_wrong_start() {
    assert_hash_bound_gap_mutation_is_damaged("gap-wrong-start", |sidecar| {
        sidecar["timelineGaps"][0]["startMs"] = serde_json::Value::from(11);
    });
}

#[test]
fn scanner_rejects_hash_bound_gap_with_wrong_source_position() {
    assert_hash_bound_gap_mutation_is_damaged("gap-wrong-source", |sidecar| {
        sidecar["timelineGaps"][0]["sourcePositionFrames"] = serde_json::Value::from(240);
    });
}

#[test]
fn scanner_rejects_hash_bound_gap_with_wrong_applicable_revision() {
    assert_hash_bound_gap_mutation_is_damaged("gap-wrong-revision", |sidecar| {
        sidecar["clockMappings"][1]["sourcePositionFrames"] = serde_json::Value::from(240);
    });
}
