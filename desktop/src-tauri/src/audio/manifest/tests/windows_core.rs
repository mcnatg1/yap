use super::*;

#[test]
fn build_manifest_windows_returns_empty_for_empty_frames() {
    assert!(windows(
        7,
        &[],
        &[],
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        window_config(false),
    )
    .is_empty());
}

#[test]
fn build_manifest_windows_uses_target_windows_for_vad_error_fallback() {
    let frames = vec![
        frame(7, 1, 0, 20, 16_000),
        frame(7, 2, 20, 20, 16_000),
        frame(7, 3, 40, 20, 16_000),
        frame(7, 4, 60, 20, 16_000),
    ];

    let chunks = windows(
        7,
        &frames,
        &[],
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        window_config(false),
    );

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].start_ms, 0);
    assert_eq!(chunks[0].duration_ms, 40);
    assert_eq!(chunks[0].vad_segments[0].kind, VadKind::Error);
    assert_eq!(chunks[0].vad_segments[0].rms, 0.0);
    assert_eq!(chunks[1].start_ms, 40);
    assert_eq!(chunks[1].duration_ms, 40);
    assert_eq!(chunks[1].vad_segments[0].kind, VadKind::Error);
}

#[test]
fn build_manifest_windows_preserves_specific_error_vad_metadata() {
    let frames = vec![
        frame(7, 1, 0, 20, 16_000),
        frame(7, 2, 20, 20, 16_000),
        frame(7, 3, 40, 20, 16_000),
    ];
    let vad = vec![VadDecision {
        kind: VadKind::Error,
        rms: 0.12,
        threshold: 0.2,
        start_ms: 0,
        end_ms: 60,
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
    assert_eq!(chunks[0].vad_segments[0].kind, VadKind::Error);
    assert_eq!(chunks[0].vad_segments[0].rms, 0.12);
    assert_eq!(chunks[1].vad_segments[0].kind, VadKind::Error);
    assert_eq!(chunks[1].vad_segments[0].rms, 0.12);
}

#[test]
fn build_manifest_windows_closes_on_vad_boundaries_before_max_window() {
    let frames = vec![
        frame(7, 1, 0, 20, 16_000),
        frame(7, 2, 20, 20, 16_000),
        frame(7, 3, 40, 20, 16_000),
        frame(7, 4, 60, 20, 16_000),
    ];
    let vad = vec![
        VadDecision {
            kind: VadKind::Speech,
            rms: 0.4,
            threshold: 0.2,
            start_ms: 0,
            end_ms: 40,
        },
        VadDecision {
            kind: VadKind::Silence,
            rms: 0.0,
            threshold: 0.2,
            start_ms: 40,
            end_ms: 80,
        },
    ];
    let mut config = window_config(false);
    config.target_window_ms = 80;
    config.max_window_ms = 120;
    config.tail_padding_ms = 0;

    let chunks = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::CaptureEnvelope,
        AudioCodec::PcmS16Le,
        config,
    );

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].start_ms, 0);
    assert_eq!(chunks[0].duration_ms, 40);
    assert_eq!(chunks[0].vad_segments.len(), 1);
    assert_eq!(chunks[0].vad_segments[0].kind, VadKind::Speech);
    assert_eq!(chunks[0].vad_segments[0].end_ms, 40);
}

#[test]
fn build_manifest_windows_adds_final_word_tail_padding_from_available_frames() {
    let frames = vec![
        frame(7, 1, 0, 20, 16_000),
        frame(7, 2, 20, 20, 16_000),
        frame(7, 3, 40, 20, 16_000),
        frame(7, 4, 60, 20, 16_000),
    ];
    let vad = vec![
        VadDecision {
            kind: VadKind::Speech,
            rms: 0.6,
            threshold: 0.2,
            start_ms: 0,
            end_ms: 40,
        },
        VadDecision {
            kind: VadKind::Silence,
            rms: 0.0,
            threshold: 0.2,
            start_ms: 40,
            end_ms: 80,
        },
    ];

    let chunks = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        window_config(false),
    );

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].duration_ms, 60);
    assert_eq!(
        chunks[0].vad_segments,
        vec![VadSegment {
            start_ms: 0,
            end_ms: 40,
            kind: VadKind::Speech,
            rms: 0.6,
        }]
    );
}

#[test]
fn build_manifest_windows_does_not_double_apply_tail_padding_when_vad_is_already_padded() {
    let frames = vec![
        frame(7, 1, 0, 20, 16_000),
        frame(7, 2, 20, 20, 16_000),
        frame(7, 3, 40, 20, 16_000),
        frame(7, 4, 60, 20, 16_000),
        frame(7, 5, 80, 20, 16_000),
    ];
    let vad = vec![
        VadDecision {
            kind: VadKind::Speech,
            rms: 0.6,
            threshold: 0.2,
            start_ms: 0,
            end_ms: 60,
        },
        VadDecision {
            kind: VadKind::Silence,
            rms: 0.0,
            threshold: 0.2,
            start_ms: 40,
            end_ms: 100,
        },
    ];

    let chunks = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        window_config(false),
    );

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].start_ms, 0);
    assert_eq!(chunks[0].duration_ms, 60);
    assert_eq!(
        chunks[0].vad_segments,
        vec![VadSegment {
            start_ms: 0,
            end_ms: 40,
            kind: VadKind::Speech,
            rms: 0.6,
        }]
    );
}

#[test]
fn build_manifest_windows_allows_speech_to_grow_to_max_window_ms() {
    let frames = vec![
        frame(7, 1, 0, 20, 16_000),
        frame(7, 2, 20, 20, 16_000),
        frame(7, 3, 40, 20, 16_000),
        frame(7, 4, 60, 20, 16_000),
        frame(7, 5, 80, 20, 16_000),
        frame(7, 6, 100, 20, 16_000),
    ];
    let vad = vec![VadDecision {
        kind: VadKind::Speech,
        rms: 0.6,
        threshold: 0.2,
        start_ms: 0,
        end_ms: 120,
    }];
    let mut config = window_config(false);
    config.tail_padding_ms = 0;

    let chunks = windows(
        7,
        &frames,
        &vad,
        AudioPurpose::LocalFallback,
        AudioCodec::PcmS16Le,
        config,
    );

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].start_ms, 0);
    assert_eq!(chunks[0].duration_ms, 80);
    assert_eq!(chunks[1].start_ms, 80);
    assert_eq!(chunks[1].duration_ms, 40);
    assert!(chunks.iter().all(|chunk| chunk.vad_segments
        == vec![VadSegment {
            start_ms: chunk.start_ms,
            end_ms: chunk.start_ms + u64::from(chunk.duration_ms),
            kind: VadKind::Speech,
            rms: 0.6,
        }]));
}
