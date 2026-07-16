use super::{
    envelope::ChunkWindowConfig,
    window_support::{
        advance_while_kind, assign_vad, build_chunk, build_error_windows, extend_speech_tail,
        has_mixed_session_or_sample_rate, max_rms, resolve_speech_boundary_ms, split_windows,
        TailExtension,
    },
};
use crate::audio::frame::{
    AudioChunkEnvelope, AudioCodec, AudioPurpose, ManifestError, PreparedFrame, VadSegment,
};
use crate::audio::vad::{VadDecision, VadKind};

pub fn build_manifest_windows(
    session_id: u64,
    frames: &[PreparedFrame],
    vad: &[VadDecision],
    purpose: AudioPurpose,
    codec: AudioCodec,
    config: ChunkWindowConfig,
) -> Result<Vec<AudioChunkEnvelope>, ManifestError> {
    if frames.is_empty() {
        return Ok(Vec::new());
    }

    let mut sorted_frames = frames.to_vec();
    sorted_frames.sort_by_key(|frame| {
        (
            frame.metadata.start_ms,
            frame.metadata.sequence,
            frame.metadata.duration_ms,
        )
    });

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
                    let assigned_speech_end_ms =
                        sorted_frames[speech_chunk_end - 1].metadata.end_ms();
                    let speech_end_ms = resolve_speech_boundary_ms(
                        vad,
                        sorted_frames[start].metadata.start_ms,
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
                        start_ms: sorted_frames[start].metadata.start_ms,
                        end_ms: speech_end_ms,
                        kind: VadKind::Speech,
                        rms: max_rms(&assignments[start..speech_chunk_end], VadKind::Speech),
                    }];

                    if config.preserve_silence_markers && chunk_end > speech_chunk_end {
                        vad_segments.push(VadSegment {
                            start_ms: speech_end_ms,
                            end_ms: sorted_frames[chunk_end - 1].metadata.end_ms(),
                            kind: VadKind::Silence,
                            rms: max_rms(
                                &assignments[speech_chunk_end..chunk_end],
                                VadKind::Silence,
                            ),
                        });
                    }

                    chunks.push(build_chunk(
                        &sorted_frames[start..chunk_end],
                        codec,
                        vad_segments,
                        purpose,
                    )?);
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
                        let chunk_end_ms = sorted_frames[end - 1].metadata.end_ms();
                        let vad_segments = vec![VadSegment {
                            start_ms: sorted_frames[start].metadata.start_ms,
                            end_ms: chunk_end_ms,
                            kind: VadKind::Silence,
                            rms: max_rms(&assignments[start..end], VadKind::Silence),
                        }];

                        chunks.push(build_chunk(
                            &sorted_frames[start..end],
                            codec,
                            vad_segments,
                            purpose,
                        )?);
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
                    let chunk_end_ms = sorted_frames[end - 1].metadata.end_ms();
                    let vad_segments = vec![VadSegment {
                        start_ms: sorted_frames[start].metadata.start_ms,
                        end_ms: chunk_end_ms,
                        kind: VadKind::Error,
                        rms: max_rms(&assignments[start..end], VadKind::Error),
                    }];

                    chunks.push(build_chunk(
                        &sorted_frames[start..end],
                        codec,
                        vad_segments,
                        purpose,
                    )?);
                }
                index = error_end;
            }
        }
    }

    Ok(chunks)
}
