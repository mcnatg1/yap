use crate::audio::frame::{AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, VadSegment};
use crate::audio::vad::{VadDecision, VadKind};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionEnvelope {
    pub session_id: u64,
    pub source: AudioSource,
    pub started_at_ms: u64,
    pub sample_rate_hz: u32,
    pub chunks: Vec<AudioChunkEnvelope>,
    pub degraded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkWindowConfig {
    pub target_window_ms: u32,
    pub max_window_ms: u32,
    pub tail_padding_ms: u32,
    pub preserve_silence_markers: bool,
}

pub struct AudioChunkEnvelopeBuilder {
    session_id: u64,
    sequence_start: Option<u64>,
    purpose: AudioPurpose,
    codec: AudioCodec,
    frames: Vec<AudioFrame>,
}

impl AudioChunkEnvelopeBuilder {
    pub fn new(session_id: u64, purpose: AudioPurpose, codec: AudioCodec) -> Self {
        Self {
            session_id,
            sequence_start: None,
            purpose,
            codec,
            frames: Vec::new(),
        }
    }

    pub fn push(&mut self, frame: AudioFrame) {
        self.sequence_start = Some(match self.sequence_start {
            Some(sequence_start) => sequence_start.min(frame.sequence),
            None => frame.sequence,
        });
        self.frames.push(frame);
    }

    pub fn finish(mut self, vad_segments: Vec<VadSegment>) -> Option<AudioChunkEnvelope> {
        let sequence_start = self.sequence_start?;
        self.frames
            .sort_by_key(|frame| (frame.sequence, frame.start_ms, frame.duration_ms));

        AudioChunkEnvelope::from_frames(
            self.session_id,
            sequence_start,
            &self.frames,
            self.codec,
            vad_segments,
            self.purpose,
        )
    }
}

pub struct AudioSessionEnvelopeBuilder {
    session_id: u64,
    source: AudioSource,
    started_at_ms: u64,
    sample_rate_hz: u32,
    chunks: Vec<AudioChunkEnvelope>,
    degraded: bool,
}

impl AudioSessionEnvelopeBuilder {
    pub fn new(
        session_id: u64,
        source: AudioSource,
        started_at_ms: u64,
        sample_rate_hz: u32,
    ) -> Self {
        Self {
            session_id,
            source,
            started_at_ms,
            sample_rate_hz,
            chunks: Vec::new(),
            degraded: false,
        }
    }

    pub fn push_chunk(&mut self, chunk: AudioChunkEnvelope) {
        self.chunks.push(chunk);
    }

    pub fn mark_degraded(&mut self) {
        self.degraded = true;
    }

    pub fn finish(mut self) -> AudioSessionEnvelope {
        self.chunks
            .sort_by_key(|chunk| (chunk.sequence_start, chunk.start_ms, chunk.chunk_id.clone()));

        AudioSessionEnvelope {
            session_id: self.session_id,
            source: self.source,
            started_at_ms: self.started_at_ms,
            sample_rate_hz: self.sample_rate_hz,
            chunks: self.chunks,
            degraded: self.degraded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    Live,
    Recording,
}

#[derive(Debug, Clone, Copy)]
struct FrameVadAssignment {
    kind: VadKind,
    rms: f32,
}

pub fn build_manifest_windows(
    session_id: u64,
    frames: &[AudioFrame],
    vad: &[VadDecision],
    purpose: AudioPurpose,
    codec: AudioCodec,
    config: ChunkWindowConfig,
) -> Vec<AudioChunkEnvelope> {
    if frames.is_empty() {
        return Vec::new();
    }

    let mut sorted_frames = frames.to_vec();
    sorted_frames.sort_by_key(|frame| (frame.start_ms, frame.sequence, frame.duration_ms));

    let target_window_ms = u64::from(config.target_window_ms.max(1));
    let max_window_ms = u64::from(config.max_window_ms.max(config.target_window_ms.max(1)));
    if has_mixed_session_or_sample_rate(session_id, &sorted_frames) {
        return build_error_windows(session_id, &sorted_frames, purpose, codec, target_window_ms);
    }

    let assignments = sorted_frames
        .iter()
        .map(|frame| assign_vad(frame, vad))
        .collect::<Vec<_>>();

    let mut chunks = Vec::new();
    let mut index = 0;
    while index < sorted_frames.len() {
        match assignments[index].kind {
            VadKind::Speech => {
                let speech_end = advance_while_kind(&assignments, index, VadKind::Speech);
                let speech_windows =
                    split_windows(&sorted_frames[index..speech_end], max_window_ms);
                let last_window = speech_windows.len().saturating_sub(1);
                let mut consumed = speech_end;

                for (window_index, (relative_start, relative_end)) in
                    speech_windows.iter().enumerate()
                {
                    let start = index + relative_start;
                    let speech_chunk_end = index + relative_end;
                    let mut chunk_end = speech_chunk_end;
                    let assigned_speech_end_ms = sorted_frames[speech_chunk_end - 1].end_ms();
                    let speech_end_ms = resolve_speech_boundary_ms(
                        vad,
                        sorted_frames[start].start_ms,
                        assigned_speech_end_ms,
                    );

                    if window_index == last_window {
                        chunk_end = extend_speech_tail(
                            &sorted_frames,
                            &assignments,
                            TailExtension {
                                chunk_start: start,
                                speech_chunk_end,
                                speech_end_ms,
                                assigned_speech_end_ms,
                                tail_padding_ms: config.tail_padding_ms,
                                max_window_ms,
                            },
                        );
                        consumed = chunk_end;
                    }

                    let mut vad_segments = vec![VadSegment {
                        start_ms: sorted_frames[start].start_ms,
                        end_ms: speech_end_ms,
                        kind: VadKind::Speech,
                        rms: max_rms(&assignments[start..speech_chunk_end], VadKind::Speech),
                    }];

                    if config.preserve_silence_markers && chunk_end > speech_chunk_end {
                        vad_segments.push(VadSegment {
                            start_ms: speech_end_ms,
                            end_ms: sorted_frames[chunk_end - 1].end_ms(),
                            kind: VadKind::Silence,
                            rms: max_rms(
                                &assignments[speech_chunk_end..chunk_end],
                                VadKind::Silence,
                            ),
                        });
                    }

                    if let Some(chunk) = build_chunk(
                        session_id,
                        &sorted_frames[start..chunk_end],
                        codec,
                        vad_segments,
                        purpose,
                    ) {
                        chunks.push(chunk);
                    }
                }

                index = consumed;
            }
            VadKind::Silence => {
                let silence_end = advance_while_kind(&assignments, index, VadKind::Silence);
                if config.preserve_silence_markers {
                    for (relative_start, relative_end) in
                        split_windows(&sorted_frames[index..silence_end], target_window_ms)
                    {
                        let start = index + relative_start;
                        let end = index + relative_end;
                        let chunk_end_ms = sorted_frames[end - 1].end_ms();
                        let vad_segments = vec![VadSegment {
                            start_ms: sorted_frames[start].start_ms,
                            end_ms: chunk_end_ms,
                            kind: VadKind::Silence,
                            rms: max_rms(&assignments[start..end], VadKind::Silence),
                        }];

                        if let Some(chunk) = build_chunk(
                            session_id,
                            &sorted_frames[start..end],
                            codec,
                            vad_segments,
                            purpose,
                        ) {
                            chunks.push(chunk);
                        }
                    }
                }
                index = silence_end;
            }
            VadKind::Error => {
                let error_end = advance_while_kind(&assignments, index, VadKind::Error);
                for (relative_start, relative_end) in
                    split_windows(&sorted_frames[index..error_end], target_window_ms)
                {
                    let start = index + relative_start;
                    let end = index + relative_end;
                    let chunk_end_ms = sorted_frames[end - 1].end_ms();
                    let vad_segments = vec![VadSegment {
                        start_ms: sorted_frames[start].start_ms,
                        end_ms: chunk_end_ms,
                        kind: VadKind::Error,
                        rms: max_rms(&assignments[start..end], VadKind::Error),
                    }];

                    if let Some(chunk) = build_chunk(
                        session_id,
                        &sorted_frames[start..end],
                        codec,
                        vad_segments,
                        purpose,
                    ) {
                        chunks.push(chunk);
                    }
                }
                index = error_end;
            }
        }
    }

    chunks
}

fn build_chunk(
    session_id: u64,
    frames: &[AudioFrame],
    codec: AudioCodec,
    vad_segments: Vec<VadSegment>,
    purpose: AudioPurpose,
) -> Option<AudioChunkEnvelope> {
    let sequence_start = frames.iter().map(|frame| frame.sequence).min()?;
    AudioChunkEnvelope::from_frames(
        session_id,
        sequence_start,
        frames,
        codec,
        vad_segments,
        purpose,
    )
}

fn build_error_windows(
    session_id: u64,
    frames: &[AudioFrame],
    purpose: AudioPurpose,
    codec: AudioCodec,
    window_ms: u64,
) -> Vec<AudioChunkEnvelope> {
    partition_identity_runs(frames)
        .into_iter()
        .flat_map(|run| {
            split_windows(run, window_ms)
                .into_iter()
                .filter_map(|(start, end)| {
                    let chunk_frames = &run[start..end];
                    let chunk_end_ms = chunk_frames.last()?.end_ms();
                    build_chunk(
                        session_id,
                        chunk_frames,
                        codec,
                        vec![VadSegment {
                            start_ms: chunk_frames.first()?.start_ms,
                            end_ms: chunk_end_ms,
                            kind: VadKind::Error,
                            rms: 0.0,
                        }],
                        purpose,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn resolve_speech_boundary_ms(
    vad: &[VadDecision],
    chunk_start_ms: u64,
    assigned_end_ms: u64,
) -> u64 {
    vad.iter()
        .filter(|decision| {
            decision.kind != VadKind::Speech
                && decision.start_ms > chunk_start_ms
                && decision.start_ms < assigned_end_ms
                && decision.end_ms > chunk_start_ms
        })
        .map(|decision| decision.start_ms)
        .min()
        .unwrap_or(assigned_end_ms)
}

fn partition_identity_runs(frames: &[AudioFrame]) -> Vec<&[AudioFrame]> {
    let mut runs = Vec::new();
    let mut start = 0;

    while start < frames.len() {
        let session_id = frames[start].session_id;
        let sample_rate_hz = frames[start].sample_rate_hz;
        let mut end = start + 1;
        while end < frames.len()
            && frames[end].session_id == session_id
            && frames[end].sample_rate_hz == sample_rate_hz
        {
            end += 1;
        }
        runs.push(&frames[start..end]);
        start = end;
    }

    runs
}

fn has_mixed_session_or_sample_rate(session_id: u64, frames: &[AudioFrame]) -> bool {
    let expected_sample_rate_hz = frames[0].sample_rate_hz;
    frames.iter().any(|frame| {
        frame.session_id != session_id || frame.sample_rate_hz != expected_sample_rate_hz
    })
}

fn assign_vad(frame: &AudioFrame, vad: &[VadDecision]) -> FrameVadAssignment {
    vad.iter()
        .filter_map(|decision| {
            let overlap_start = frame.start_ms.max(decision.start_ms);
            let overlap_end = frame.end_ms().min(decision.end_ms);
            (overlap_end > overlap_start).then(|| {
                (
                    overlap_end - overlap_start,
                    vad_priority(decision.kind),
                    *decision,
                )
            })
        })
        .max_by_key(|(overlap_ms, priority, _)| (*overlap_ms, *priority))
        .map(|(_, _, decision)| FrameVadAssignment {
            kind: decision.kind,
            rms: decision.rms,
        })
        .unwrap_or(FrameVadAssignment {
            kind: VadKind::Error,
            rms: 0.0,
        })
}

fn vad_priority(kind: VadKind) -> u8 {
    match kind {
        VadKind::Error => 0,
        VadKind::Silence => 1,
        VadKind::Speech => 2,
    }
}

fn advance_while_kind(assignments: &[FrameVadAssignment], start: usize, kind: VadKind) -> usize {
    let mut end = start + 1;
    while end < assignments.len() && assignments[end].kind == kind {
        end += 1;
    }
    end
}

fn split_windows(frames: &[AudioFrame], window_ms: u64) -> Vec<(usize, usize)> {
    if frames.is_empty() {
        return Vec::new();
    }

    let mut windows = Vec::new();
    let mut start = 0;
    let window_ms = window_ms.max(1);

    while start < frames.len() {
        let chunk_start_ms = frames[start].start_ms;
        let mut end = start + 1;
        while end < frames.len() {
            let candidate_end_ms = frames[end].end_ms();
            let candidate_duration_ms = candidate_end_ms.saturating_sub(chunk_start_ms);
            if candidate_duration_ms > window_ms {
                break;
            }
            end += 1;
        }

        windows.push((start, end));
        start = end;
    }

    windows
}

struct TailExtension {
    chunk_start: usize,
    speech_chunk_end: usize,
    speech_end_ms: u64,
    assigned_speech_end_ms: u64,
    tail_padding_ms: u32,
    max_window_ms: u64,
}

fn extend_speech_tail(
    frames: &[AudioFrame],
    assignments: &[FrameVadAssignment],
    tail: TailExtension,
) -> usize {
    if tail.tail_padding_ms == 0 {
        return tail.speech_chunk_end;
    }

    let already_covered_tail_ms = tail
        .assigned_speech_end_ms
        .saturating_sub(tail.speech_end_ms);
    let remaining_tail_ms = u64::from(tail.tail_padding_ms).saturating_sub(already_covered_tail_ms);
    if remaining_tail_ms == 0 {
        return tail.speech_chunk_end;
    }

    let allowed_tail_end_ms = tail
        .assigned_speech_end_ms
        .saturating_add(remaining_tail_ms);
    let allowed_chunk_end_ms = frames[tail.chunk_start]
        .start_ms
        .saturating_add(tail.max_window_ms);
    let final_allowed_end_ms = allowed_tail_end_ms.min(allowed_chunk_end_ms);

    let mut end = tail.speech_chunk_end;
    while end < frames.len()
        && assignments[end].kind == VadKind::Silence
        && frames[end].end_ms() <= final_allowed_end_ms
    {
        end += 1;
    }

    end
}

fn max_rms(assignments: &[FrameVadAssignment], kind: VadKind) -> f32 {
    assignments
        .iter()
        .filter(|assignment| assignment.kind == kind)
        .map(|assignment| assignment.rms)
        .fold(0.0_f32, f32::max)
}

#[cfg(test)]
mod tests {
    use super::{
        build_manifest_windows, AudioChunkEnvelopeBuilder, AudioSessionEnvelope,
        AudioSessionEnvelopeBuilder, AudioSource, ChunkWindowConfig,
    };
    use crate::audio::frame::{
        AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, RetryMetadata, VadSegment,
    };
    use crate::audio::vad::{VadDecision, VadKind};

    fn frame(
        session_id: u64,
        sequence: u64,
        start_ms: u64,
        duration_ms: u32,
        sample_rate_hz: u32,
    ) -> AudioFrame {
        AudioFrame {
            session_id,
            sequence,
            sample_rate_hz,
            channels: 1,
            start_ms,
            duration_ms,
            sample_count: 320,
        }
    }

    #[test]
    fn session_envelope_serializes_with_expected_field_names() {
        let session = AudioSessionEnvelope {
            session_id: 55,
            source: AudioSource::Live,
            started_at_ms: 1_000,
            sample_rate_hz: 16_000,
            chunks: vec![AudioChunkEnvelope {
                session_id: 55,
                chunk_id: "55-2-40".into(),
                sequence_start: 2,
                start_ms: 40,
                duration_ms: 40,
                sample_rate_hz: 16_000,
                codec: AudioCodec::PcmS16Le,
                vad_segments: vec![VadSegment {
                    start_ms: 40,
                    end_ms: 80,
                    kind: VadKind::Speech,
                    rms: 0.33,
                }],
                purpose: AudioPurpose::CaptureEnvelope,
                retry: RetryMetadata {
                    idempotency_key: "55-2-55-2-40".into(),
                    attempt: 1,
                    max_attempts: 1,
                },
            }],
            degraded: true,
        };

        let value = serde_json::to_value(&session).expect("session envelope should serialize");

        assert_eq!(value["sessionId"], 55);
        assert_eq!(value["source"], "live");
        assert_eq!(value["startedAtMs"], 1_000);
        assert_eq!(value["sampleRateHz"], 16_000);
        assert_eq!(value["degraded"], true);
        assert_eq!(value["chunks"][0]["chunkId"], "55-2-40");
    }

    #[test]
    fn chunk_builder_orders_contiguous_frames_by_sequence() {
        let mut builder =
            AudioChunkEnvelopeBuilder::new(7, AudioPurpose::CaptureEnvelope, AudioCodec::PcmS16Le);
        builder.push(frame(7, 12, 120, 20, 16_000));
        builder.push(frame(7, 11, 100, 20, 16_000));

        let vad_segments = vec![VadSegment {
            start_ms: 100,
            end_ms: 140,
            kind: VadKind::Speech,
            rms: 0.42,
        }];

        let envelope = builder
            .finish(vad_segments.clone())
            .expect("frames should build an envelope");

        assert_eq!(envelope.session_id, 7);
        assert_eq!(envelope.chunk_id, "7-11-40");
        assert_eq!(envelope.sequence_start, 11);
        assert_eq!(envelope.start_ms, 100);
        assert_eq!(envelope.duration_ms, 40);
        assert_eq!(envelope.sample_rate_hz, 16_000);
        assert_eq!(envelope.codec, AudioCodec::PcmS16Le);
        assert_eq!(envelope.vad_segments, vad_segments);
        assert_eq!(envelope.purpose, AudioPurpose::CaptureEnvelope);
    }

    #[test]
    fn chunk_builder_returns_none_for_empty_builders() {
        let builder =
            AudioChunkEnvelopeBuilder::new(7, AudioPurpose::LocalFallback, AudioCodec::PcmS16Le);

        assert!(builder.finish(Vec::new()).is_none());
    }

    #[test]
    fn chunk_builder_sets_retry_and_idempotency_fields() {
        let mut builder =
            AudioChunkEnvelopeBuilder::new(7, AudioPurpose::LocalFallback, AudioCodec::PcmS16Le);
        builder.push(frame(7, 11, 100, 20, 16_000));
        builder.push(frame(7, 12, 120, 20, 16_000));

        let envelope = builder
            .finish(vec![VadSegment {
                start_ms: 100,
                end_ms: 140,
                kind: VadKind::Speech,
                rms: 0.42,
            }])
            .expect("frames should build an envelope");

        assert_eq!(
            envelope.retry,
            RetryMetadata {
                idempotency_key: "7-11-7-11-40".into(),
                attempt: 1,
                max_attempts: 1,
            }
        );
    }

    #[test]
    fn session_builder_collects_chunks_and_marks_degraded() {
        let mut first_chunk_builder =
            AudioChunkEnvelopeBuilder::new(55, AudioPurpose::CaptureEnvelope, AudioCodec::PcmS16Le);
        first_chunk_builder.push(frame(55, 4, 80, 20, 16_000));
        first_chunk_builder.push(frame(55, 5, 100, 20, 16_000));
        let first_chunk = first_chunk_builder
            .finish(vec![VadSegment {
                start_ms: 80,
                end_ms: 120,
                kind: VadKind::Silence,
                rms: 0.0,
            }])
            .expect("first chunk should build");

        let mut second_chunk_builder =
            AudioChunkEnvelopeBuilder::new(55, AudioPurpose::CaptureEnvelope, AudioCodec::PcmS16Le);
        second_chunk_builder.push(frame(55, 2, 40, 20, 16_000));
        second_chunk_builder.push(frame(55, 3, 60, 20, 16_000));
        let second_chunk = second_chunk_builder
            .finish(vec![VadSegment {
                start_ms: 40,
                end_ms: 80,
                kind: VadKind::Speech,
                rms: 0.33,
            }])
            .expect("second chunk should build");

        let mut session_builder =
            AudioSessionEnvelopeBuilder::new(55, AudioSource::Live, 1_000, 16_000);
        session_builder.push_chunk(first_chunk);
        session_builder.push_chunk(second_chunk);
        session_builder.mark_degraded();

        let session = session_builder.finish();

        assert_eq!(session.session_id, 55);
        assert_eq!(session.source, AudioSource::Live);
        assert_eq!(session.started_at_ms, 1_000);
        assert_eq!(session.sample_rate_hz, 16_000);
        assert!(session.degraded);
        assert_eq!(session.chunks.len(), 2);
        assert_eq!(session.chunks[0].chunk_id, "55-2-40");
        assert_eq!(session.chunks[1].chunk_id, "55-4-40");
    }

    fn window_config(preserve_silence_markers: bool) -> ChunkWindowConfig {
        ChunkWindowConfig {
            target_window_ms: 40,
            max_window_ms: 80,
            tail_padding_ms: 20,
            preserve_silence_markers,
        }
    }

    #[test]
    fn build_manifest_windows_returns_empty_for_empty_frames() {
        assert!(build_manifest_windows(
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

        let chunks = build_manifest_windows(
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

        let chunks = build_manifest_windows(
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

        let chunks = build_manifest_windows(
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

        let chunks = build_manifest_windows(
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

        let chunks = build_manifest_windows(
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

        let chunks = build_manifest_windows(
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

        let dropped = build_manifest_windows(
            7,
            &frames,
            &vad,
            AudioPurpose::LocalFallback,
            AudioCodec::PcmS16Le,
            window_config(false),
        );
        let preserved = build_manifest_windows(
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

        let chunks = build_manifest_windows(
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
}
