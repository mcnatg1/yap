use super::*;

#[test]
fn build_manifest_windows_preserves_silence_markers_only_when_requested() {
    let frames = vec![
        frame(7, 1, 0, 20, 16_000),
        frame(7, 2, 20, 20, 16_000),
        frame(7, 3, 40, 20, 16_000),
    ];
    let vad = vec![VadDecision {
        kind: VadKind::Silence,
        rms: 0.0,
        threshold: 0.2,
        start_ms: 0,
        end_ms: 60,
    }];

    let dropped = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        window_config(false),
    );
    let preserved = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        window_config(true),
    );

    assert!(dropped.is_empty());
    assert_eq!(preserved.len(), 2);
    assert_eq!(preserved[0].vad_segments[0].kind, VadKind::Silence);
    assert_eq!(preserved[0].duration_ms, 40);
    assert_eq!(preserved[1].vad_segments[0].kind, VadKind::Silence);
    assert_eq!(preserved[1].duration_ms, 20);
}

#[test]
fn build_manifest_windows_marks_mixed_sample_rates_as_error_chunks() {
    let frames = vec![frame(7, 1, 0, 20, 16_000), frame(7, 2, 20, 20, 8_000)];
    let vad = vec![VadDecision {
        kind: VadKind::Speech,
        rms: 0.6,
        threshold: 0.2,
        start_ms: 0,
        end_ms: 40,
    }];

    let chunks = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        window_config(false),
    );

    assert_eq!(chunks.len(), 2);
    assert!(chunks.iter().all(|chunk| chunk.vad_segments
        == vec![VadSegment {
            start_ms: chunk.start_ms,
            end_ms: chunk.start_ms + u64::from(chunk.duration_ms),
            kind: VadKind::Error,
            rms: 0.0,
        }]));
}

#[test]
fn build_manifest_windows_hashes_retained_pcm_samples() {
    let metadata = frame(7, 1, 0, 20, 16_000);
    let first = [prepared_frame(metadata.clone(), &[0.0, 0.5])];
    let second = [prepared_frame(metadata, &[0.0, 0.25])];
    let vad = [VadDecision {
        kind: VadKind::Speech,
        rms: 0.5,
        threshold: 0.2,
        start_ms: 0,
        end_ms: 20,
    }];

    let first_chunk = build_manifest_windows(
        7,
        &first,
        &vad,
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
        window_config(false),
    )
    .unwrap()
    .remove(0);
    let second_chunk = build_manifest_windows(
        7,
        &second,
        &vad,
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
        window_config(false),
    )
    .unwrap()
    .remove(0);

    assert!(first_chunk.content_identity.byte_length > 0);
    assert_ne!(first_chunk.content_identity, second_chunk.content_identity);
    assert!(matches!(
        super::classify_replay(
            &first_chunk.replay_key,
            &first_chunk.content_identity,
            &second_chunk.replay_key,
            &second_chunk.content_identity,
        ),
        Err(super::ReplayConflict::SameKeyDifferentContent)
    ));
}

#[test]
fn window_building_splits_rate_changes_and_persisted_revision_authorizes_chunks() {
    let frames = vec![frame(7, 1, 0, 20, 16_000), frame(7, 2, 20, 20, 8_000)];
    let vad = vec![VadDecision {
        kind: VadKind::Speech,
        rms: 0.6,
        threshold: 0.2,
        start_ms: 0,
        end_ms: 40,
    }];
    let chunks = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
        window_config(false),
    );

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.sample_rate_hz)
            .collect::<Vec<_>>(),
        [16_000, 8_000]
    );
    let mut session = AudioSessionEnvelopeBuilder::new(
        session_id(7),
        SessionMode::Dictation,
        SessionOrigin::LiveCapture,
        vec![track_descriptor()],
        0,
        16_000,
    );
    session.push_track_configuration_revision(
        crate::audio::frame::TrackConfigurationRevision::new(track_id(), 1, 20, 8_000).unwrap(),
    );
    for chunk in chunks {
        session.push_chunk(chunk);
    }

    assert!(session.finish().is_ok());
}

#[test]
fn internal_exact_gap_round_trips_without_covering_retained_audio() {
    let owner = crate::audio::session::OwnerNamespace::local("install-1").unwrap();
    let track = track_descriptor();
    let gap = AudioGap {
        session_id: session_id(7),
        track_id: track_id(),
        start_ms: 20,
        duration_ms: 20,
        source_position_frames: 320,
        dropped_frames: 320,
        cause: GapCause::SinkUnavailable,
        generation: 1,
    };
    let chunk = crate::audio::frame::AudioChunkEnvelope::from_frames_with_continuity(
        session_id(7),
        chunk_context(&owner, &track, b"audio"),
        &[frame(7, 1, 0, 20, 16_000), frame(7, 3, 40, 20, 16_000)],
        AudioCodec::PcmS16Le,
        Vec::new(),
        vec![gap],
        AudioPurpose::CaptureEnvelope,
    )
    .unwrap()
    .capture_descriptor();
    let session = AudioSessionEnvelope {
        schema_version: MANIFEST_SCHEMA_VERSION,
        session_id: session_id(7),
        session_mode: SessionMode::Dictation,
        session_origin: SessionOrigin::LiveCapture,
        tracks: vec![track],
        track_configuration_revisions: Vec::new(),
        started_at_ms: 0,
        sample_rate_hz: 16_000,
        chunks: vec![chunk],
        degraded: true,
    };

    assert!(
        serde_json::from_value::<AudioSessionEnvelope>(serde_json::to_value(session).unwrap())
            .is_ok()
    );
}
