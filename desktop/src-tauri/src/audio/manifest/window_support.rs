use crate::audio::frame::{
    AudioChunkEnvelope, AudioCodec, AudioPurpose, AudioRoute, ChunkBuildContext, ManifestError,
    PreparedFrame, VadSegment,
};
use crate::audio::session::{
    CaptureSource, CaptureTrackDescriptor, OwnerNamespace, SessionId, SessionMode, SessionOrigin,
    TrackSource,
};
use crate::audio::vad::{VadDecision, VadKind};

#[derive(Debug, Clone, Copy)]
pub(super) struct FrameVadAssignment {
    pub(super) kind: VadKind,
    pub(super) rms: f32,
}

pub(super) fn build_chunk(
    frames: &[PreparedFrame],
    codec: AudioCodec,
    vad_segments: Vec<VadSegment>,
    purpose: AudioPurpose,
) -> Result<AudioChunkEnvelope, ManifestError> {
    let first = frames.first().ok_or(ManifestError::EmptyFrames)?;
    let owner_namespace =
        OwnerNamespace::local("legacy-window").expect("static legacy owner namespace is valid");
    let track = CaptureTrackDescriptor {
        track_id: first.metadata.track_id.clone(),
        source: TrackSource::Captured {
            source: CaptureSource::Microphone,
        },
        device_id: "dev-legacy-window".into(),
    };
    let frame_metadata = frames
        .iter()
        .map(|frame| frame.metadata.clone())
        .collect::<Vec<_>>();
    let samples = frames
        .iter()
        .flat_map(|frame| frame.samples.iter().copied())
        .collect::<Vec<_>>();
    let encoded_audio = crate::audio::preprocess::f32_to_i16_le_bytes(&samples);
    AudioChunkEnvelope::from_frames(
        first.metadata.session_id.clone(),
        ChunkBuildContext {
            owner_namespace: &owner_namespace,
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::LiveCapture,
            track: &track,
            route: AudioRoute::LocalFallback,
            audio_artifact_id: "legacy-window",
            encoded_audio: &encoded_audio,
        },
        &frame_metadata,
        codec,
        vad_segments,
        purpose,
    )
}

pub(super) fn build_error_windows(
    _session_id: u64,
    frames: &[PreparedFrame],
    purpose: AudioPurpose,
    codec: AudioCodec,
    window_ms: u64,
) -> Result<Vec<AudioChunkEnvelope>, ManifestError> {
    let mut chunks = Vec::new();
    for run in partition_identity_runs(frames) {
        for (start, end) in split_windows(run, window_ms) {
            let chunk_frames = &run[start..end];
            let first = chunk_frames.first().ok_or(ManifestError::EmptyFrames)?;
            let chunk_end_ms = chunk_frames
                .last()
                .ok_or(ManifestError::EmptyFrames)?
                .metadata
                .checked_end_ms()?;
            chunks.push(build_chunk(
                chunk_frames,
                codec,
                vec![VadSegment {
                    start_ms: first.metadata.start_ms,
                    end_ms: chunk_end_ms,
                    kind: VadKind::Error,
                    rms: 0.0,
                }],
                purpose,
            )?);
        }
    }
    Ok(chunks)
}

pub(super) fn resolve_speech_boundary_ms(
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

fn partition_identity_runs(frames: &[PreparedFrame]) -> Vec<&[PreparedFrame]> {
    let mut runs = Vec::new();
    let mut start = 0;

    while start < frames.len() {
        let session_id = frames[start].metadata.session_id.clone();
        let sample_rate_hz = frames[start].metadata.sample_rate_hz;
        let mut end = start + 1;
        while end < frames.len()
            && frames[end].metadata.session_id == session_id
            && frames[end].metadata.sample_rate_hz == sample_rate_hz
        {
            end += 1;
        }
        runs.push(&frames[start..end]);
        start = end;
    }

    runs
}

pub(super) fn has_mixed_session_or_sample_rate(session_id: u64, frames: &[PreparedFrame]) -> bool {
    let expected_sample_rate_hz = frames[0].metadata.sample_rate_hz;
    let expected_session_id =
        SessionId::new(format!("s-{session_id}")).expect("legacy numeric session ID is valid");
    frames.iter().any(|frame| {
        frame.metadata.session_id != expected_session_id
            || frame.metadata.sample_rate_hz != expected_sample_rate_hz
    })
}

pub(super) fn assign_vad(frame: &PreparedFrame, vad: &[VadDecision]) -> FrameVadAssignment {
    vad.iter()
        .filter_map(|decision| {
            let overlap_start = frame.metadata.start_ms.max(decision.start_ms);
            let overlap_end = frame.metadata.end_ms().min(decision.end_ms);
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

pub(super) fn advance_while_kind(
    assignments: &[FrameVadAssignment],
    start: usize,
    kind: VadKind,
) -> usize {
    let mut end = start + 1;
    while end < assignments.len() && assignments[end].kind == kind {
        end += 1;
    }
    end
}

pub(super) fn split_windows(frames: &[PreparedFrame], window_ms: u64) -> Vec<(usize, usize)> {
    if frames.is_empty() {
        return Vec::new();
    }

    let mut windows = Vec::new();
    let mut start = 0;
    let window_ms = window_ms.max(1);

    while start < frames.len() {
        let chunk_start_ms = frames[start].metadata.start_ms;
        let mut end = start + 1;
        while end < frames.len() {
            let candidate_end_ms = frames[end].metadata.end_ms();
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

pub(super) struct TailExtension {
    pub(super) chunk_start: usize,
    pub(super) speech_chunk_end: usize,
    pub(super) speech_end_ms: u64,
    pub(super) assigned_speech_end_ms: u64,
    pub(super) tail_padding_ms: u32,
    pub(super) max_window_ms: u64,
}

pub(super) fn extend_speech_tail(
    frames: &[PreparedFrame],
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
        .metadata
        .start_ms
        .saturating_add(tail.max_window_ms);
    let final_allowed_end_ms = allowed_tail_end_ms.min(allowed_chunk_end_ms);

    let mut end = tail.speech_chunk_end;
    while end < frames.len()
        && assignments[end].kind == VadKind::Silence
        && frames[end].metadata.end_ms() <= final_allowed_end_ms
    {
        end += 1;
    }

    end
}

pub(super) fn max_rms(assignments: &[FrameVadAssignment], kind: VadKind) -> f32 {
    assignments
        .iter()
        .filter(|assignment| assignment.kind == kind)
        .map(|assignment| assignment.rms)
        .fold(0.0_f32, f32::max)
}
