use crate::audio::frame::{
    AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, AudioRoute, CaptureChunkDescriptor,
    ChunkBuildContext, ChunkReplayKey, ContentIdentity, ManifestError, VadSegment,
};
use crate::audio::session::{
    CaptureSource, CaptureTrackDescriptor, ImportedTrackProvenance, OwnerNamespace, SessionId,
    SessionMode, SessionOrigin, TrackId, TrackSource,
};
use crate::audio::vad::{VadDecision, VadKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayDecision {
    Idempotent,
    Distinct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayConflict {
    SameKeyDifferentContent,
}

pub fn classify_replay(
    existing_key: &ChunkReplayKey,
    existing_content: &ContentIdentity,
    incoming_key: &ChunkReplayKey,
    incoming_content: &ContentIdentity,
) -> Result<ReplayDecision, ReplayConflict> {
    if existing_key != incoming_key {
        return Ok(ReplayDecision::Distinct);
    }
    if existing_content == incoming_content {
        Ok(ReplayDecision::Idempotent)
    } else {
        Err(ReplayConflict::SameKeyDifferentContent)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionEnvelope {
    pub session_id: SessionId,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub tracks: Vec<CaptureTrackDescriptor>,
    pub started_at_ms: u64,
    pub sample_rate_hz: u32,
    pub chunks: Vec<CaptureChunkDescriptor>,
    pub degraded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkWindowConfig {
    pub target_window_ms: u32,
    pub max_window_ms: u32,
    pub tail_padding_ms: u32,
    pub preserve_silence_markers: bool,
}

pub struct AudioChunkEnvelopeBuilder<'a> {
    session_id: SessionId,
    context: ChunkBuildContext<'a>,
    purpose: AudioPurpose,
    codec: AudioCodec,
    frames: Vec<AudioFrame>,
}

impl<'a> AudioChunkEnvelopeBuilder<'a> {
    pub fn new(
        session_id: SessionId,
        context: ChunkBuildContext<'a>,
        purpose: AudioPurpose,
        codec: AudioCodec,
    ) -> Self {
        Self {
            session_id,
            context,
            purpose,
            codec,
            frames: Vec::new(),
        }
    }

    pub fn push(&mut self, frame: AudioFrame) -> Result<(), ManifestError> {
        let mut candidate = self.frames.clone();
        candidate.push(frame.clone());
        AudioChunkEnvelope::from_frames(
            self.session_id.clone(),
            ChunkBuildContext {
                owner_namespace: self.context.owner_namespace,
                session_mode: self.context.session_mode,
                session_origin: self.context.session_origin,
                track: self.context.track,
                route: self.context.route,
                audio_artifact_id: self.context.audio_artifact_id,
                encoded_audio: self.context.encoded_audio,
            },
            &candidate,
            self.codec,
            Vec::new(),
            self.purpose,
        )?;
        self.frames.push(frame);
        Ok(())
    }

    pub fn finish(
        self,
        vad_segments: Vec<VadSegment>,
    ) -> Result<AudioChunkEnvelope, ManifestError> {
        AudioChunkEnvelope::from_frames(
            self.session_id,
            self.context,
            &self.frames,
            self.codec,
            vad_segments,
            self.purpose,
        )
    }
}

pub struct AudioSessionEnvelopeBuilder {
    session_id: SessionId,
    session_mode: SessionMode,
    session_origin: SessionOrigin,
    tracks: Vec<CaptureTrackDescriptor>,
    started_at_ms: u64,
    sample_rate_hz: u32,
    chunks: Vec<CaptureChunkDescriptor>,
    degraded: bool,
}

impl AudioSessionEnvelopeBuilder {
    pub fn new(
        session_id: SessionId,
        session_mode: SessionMode,
        session_origin: SessionOrigin,
        tracks: Vec<CaptureTrackDescriptor>,
        started_at_ms: u64,
        sample_rate_hz: u32,
    ) -> Self {
        Self {
            session_id,
            session_mode,
            session_origin,
            tracks,
            started_at_ms,
            sample_rate_hz,
            chunks: Vec::new(),
            degraded: false,
        }
    }

    pub fn push_chunk(&mut self, chunk: AudioChunkEnvelope) {
        self.chunks.push(chunk.capture_descriptor());
    }

    pub fn mark_degraded(&mut self) {
        self.degraded = true;
    }

    pub fn finish(mut self) -> Result<AudioSessionEnvelope, ManifestError> {
        validate_track_sources(self.session_origin, &self.tracks)
            .map_err(|_| ManifestError::SessionMetadataMismatch)?;
        self.chunks.sort_by_key(|chunk| {
            (
                chunk.track_id.as_str().to_owned(),
                chunk.sequence_start,
                chunk.start_ms,
                chunk.chunk_id.clone(),
            )
        });
        validate_chunk_references(
            &self.session_id,
            self.session_mode,
            self.session_origin,
            &self.tracks,
            &self.chunks,
        )?;

        Ok(AudioSessionEnvelope {
            session_id: self.session_id,
            session_mode: self.session_mode,
            session_origin: self.session_origin,
            tracks: self.tracks,
            started_at_ms: self.started_at_ms,
            sample_rate_hz: self.sample_rate_hz,
            chunks: self.chunks,
            degraded: self.degraded,
        })
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioSessionEnvelopeWire {
    session_id: LegacySessionId,
    #[serde(default)]
    session_mode: Option<SessionMode>,
    #[serde(default)]
    session_origin: Option<SessionOrigin>,
    #[serde(default)]
    tracks: Option<Vec<CaptureTrackDescriptor>>,
    #[serde(default)]
    source: Option<LegacyAudioSource>,
    #[serde(default)]
    started_at_ms: u64,
    #[serde(default)]
    sample_rate_hz: u32,
    #[serde(default)]
    chunks: Vec<CaptureChunkDescriptor>,
    #[serde(default)]
    degraded: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum LegacySessionId {
    Current(SessionId),
    Numeric(u64),
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyAudioSource {
    Live,
    Recording,
}

impl<'de> serde::Deserialize<'de> for AudioSessionEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AudioSessionEnvelopeWire::deserialize(deserializer)?;
        let session_id = match wire.session_id {
            LegacySessionId::Current(session_id) => session_id,
            LegacySessionId::Numeric(value) => {
                SessionId::new(format!("legacy-{value}")).map_err(serde::de::Error::custom)?
            }
        };
        let legacy_origin = wire.source.map(|source| match source {
            LegacyAudioSource::Live => SessionOrigin::LiveCapture,
            LegacyAudioSource::Recording => SessionOrigin::ImportedFile,
        });
        let session_origin = wire
            .session_origin
            .or(legacy_origin)
            .ok_or_else(|| serde::de::Error::missing_field("sessionOrigin"))?;
        let tracks = wire
            .tracks
            .unwrap_or_else(|| vec![legacy_track_descriptor(session_origin)]);
        validate_track_sources(session_origin, &tracks).map_err(serde::de::Error::custom)?;

        validate_chunk_references(
            &session_id,
            wire.session_mode.unwrap_or(SessionMode::Dictation),
            session_origin,
            &tracks,
            &wire.chunks,
        )
        .map_err(serde::de::Error::custom)?;

        Ok(Self {
            session_id,
            session_mode: wire.session_mode.unwrap_or(SessionMode::Dictation),
            session_origin,
            tracks,
            started_at_ms: wire.started_at_ms,
            sample_rate_hz: wire.sample_rate_hz,
            chunks: wire.chunks,
            degraded: wire.degraded,
        })
    }
}

fn validate_track_sources(
    session_origin: SessionOrigin,
    tracks: &[CaptureTrackDescriptor],
) -> Result<(), String> {
    let expected = match session_origin {
        SessionOrigin::LiveCapture => "captured",
        SessionOrigin::ImportedFile => "imported",
    };
    if tracks.iter().all(|track| {
        matches!(
            (session_origin, &track.source),
            (SessionOrigin::LiveCapture, TrackSource::Captured { .. })
                | (SessionOrigin::ImportedFile, TrackSource::Imported { .. })
        )
    }) {
        return Ok(());
    }
    Err(format!(
        "{session_origin:?} sessions must contain only {expected} tracks"
    ))
}

fn validate_chunk_references(
    session_id: &SessionId,
    session_mode: SessionMode,
    session_origin: SessionOrigin,
    tracks: &[CaptureTrackDescriptor],
    chunks: &[CaptureChunkDescriptor],
) -> Result<(), ManifestError> {
    for chunk in chunks {
        if chunk.session_id != *session_id || chunk.replay_key.session_id != *session_id {
            return Err(ManifestError::SessionMismatch);
        }
        if chunk.track_id != chunk.replay_key.track_id
            || !tracks
                .iter()
                .any(|track| track.track_id == chunk.track_id && track.source == chunk.track_source)
        {
            return Err(ManifestError::SessionTrackReferenceMismatch);
        }
        if chunk.session_mode != session_mode || chunk.session_origin != session_origin {
            return Err(ManifestError::SessionMetadataMismatch);
        }
        if chunk.replay_key.sequence_start != chunk.sequence_start
            || chunk.replay_key.sequence_end != chunk.sequence_end
            || !chunk.content_identity.is_valid_sha256()
            || chunk.audio_artifact_id.is_empty()
        {
            return Err(ManifestError::SessionTrackReferenceMismatch);
        }
        if chunk.replay_key.schema_version == crate::audio::frame::CHUNK_SCHEMA_VERSION
            && chunk.chunk_id
                != format!(
                    "chunk-v{}-{}-{}-{}-{}-{}",
                    chunk.replay_key.schema_version,
                    chunk.replay_key.owner_namespace,
                    chunk.replay_key.session_id,
                    chunk.replay_key.track_id,
                    chunk.replay_key.sequence_start,
                    chunk.replay_key.sequence_end,
                )
        {
            return Err(ManifestError::SessionTrackReferenceMismatch);
        }
        if chunk.sequence_end < chunk.sequence_start || chunk.duration_ms == 0 {
            return Err(ManifestError::InvalidFrameTiming);
        }
        let chunk_end_ms = chunk
            .start_ms
            .checked_add(u64::from(chunk.duration_ms))
            .ok_or(ManifestError::DurationOverflow)?;
        if chunk.vad_segments.iter().any(|segment| {
            segment.end_ms <= segment.start_ms
                || segment.start_ms < chunk.start_ms
                || segment.end_ms > chunk_end_ms
        }) {
            return Err(ManifestError::InvalidVadTiming);
        }
        if chunk.gaps.iter().any(|gap| {
            gap.session_id != *session_id
                || gap.track_id != chunk.track_id
                || gap.duration_ms == 0
                || gap.end_ms().is_none()
        }) {
            return Err(ManifestError::InvalidGapTiming);
        }
    }

    for track in tracks {
        let mut previous: Option<&CaptureChunkDescriptor> = None;
        for chunk in chunks
            .iter()
            .filter(|chunk| chunk.track_id == track.track_id)
        {
            if let Some(previous) = previous {
                let previous_end_ms = previous
                    .start_ms
                    .checked_add(u64::from(previous.duration_ms))
                    .ok_or(ManifestError::DurationOverflow)?;
                if chunk.sequence_start <= previous.sequence_end {
                    return Err(ManifestError::SequenceDiscontinuity);
                }
                if chunk.start_ms < previous_end_ms {
                    return Err(ManifestError::OverlappingFrameTiming);
                }
                let sequence_is_contiguous = chunk.sequence_start == previous.sequence_end + 1;
                let timing_is_contiguous = chunk.start_ms == previous_end_ms;
                if (!sequence_is_contiguous || !timing_is_contiguous)
                    && !chunk.gaps.iter().any(|gap| {
                        gap.session_id == *session_id
                            && gap.track_id == track.track_id
                            && gap.start_ms <= previous_end_ms
                            && gap
                                .end_ms()
                                .is_some_and(|gap_end| gap_end >= chunk.start_ms)
                    })
                {
                    return Err(if !sequence_is_contiguous {
                        ManifestError::SequenceDiscontinuity
                    } else {
                        ManifestError::TimingDiscontinuity
                    });
                }
            }
            previous = Some(chunk);
        }
    }
    Ok(())
}

fn legacy_track_descriptor(origin: SessionOrigin) -> CaptureTrackDescriptor {
    let source = match origin {
        SessionOrigin::LiveCapture => TrackSource::Captured {
            source: CaptureSource::Microphone,
        },
        SessionOrigin::ImportedFile => TrackSource::Imported {
            provenance: ImportedTrackProvenance::Unknown,
        },
    };
    CaptureTrackDescriptor {
        track_id: TrackId::new("legacy-0").expect("static legacy track ID is valid"),
        source,
        device_id: "dev-legacy".into(),
    }
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
) -> Result<Vec<AudioChunkEnvelope>, ManifestError> {
    if frames.is_empty() {
        return Ok(Vec::new());
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
                        let chunk_end_ms = sorted_frames[end - 1].end_ms();
                        let vad_segments = vec![VadSegment {
                            start_ms: sorted_frames[start].start_ms,
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
                    let chunk_end_ms = sorted_frames[end - 1].end_ms();
                    let vad_segments = vec![VadSegment {
                        start_ms: sorted_frames[start].start_ms,
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

fn build_chunk(
    frames: &[AudioFrame],
    codec: AudioCodec,
    vad_segments: Vec<VadSegment>,
    purpose: AudioPurpose,
) -> Result<AudioChunkEnvelope, ManifestError> {
    let first = frames.first().ok_or(ManifestError::EmptyFrames)?;
    let owner_namespace =
        OwnerNamespace::local("legacy-window").expect("static legacy owner namespace is valid");
    let track = CaptureTrackDescriptor {
        track_id: first.track_id.clone(),
        source: TrackSource::Captured {
            source: CaptureSource::Microphone,
        },
        device_id: "dev-legacy-window".into(),
    };
    AudioChunkEnvelope::from_frames(
        first.session_id.clone(),
        ChunkBuildContext {
            owner_namespace: &owner_namespace,
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::LiveCapture,
            track: &track,
            route: AudioRoute::LocalFallback,
            audio_artifact_id: "legacy-window",
            encoded_audio: &[],
        },
        frames,
        codec,
        vad_segments,
        purpose,
    )
}

fn build_error_windows(
    _session_id: u64,
    frames: &[AudioFrame],
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
                .checked_end_ms()?;
            chunks.push(build_chunk(
                chunk_frames,
                codec,
                vec![VadSegment {
                    start_ms: first.start_ms,
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
        let session_id = frames[start].session_id.clone();
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
    let expected_session_id =
        SessionId::new(format!("s-{session_id}")).expect("legacy numeric session ID is valid");
    frames.iter().any(|frame| {
        frame.session_id != expected_session_id || frame.sample_rate_hz != expected_sample_rate_hz
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
        AudioSessionEnvelopeBuilder, ChunkWindowConfig,
    };
    use crate::audio::frame::{
        AudioCodec, AudioFrame, AudioPurpose, AudioRoute, ChunkBuildContext, VadSegment,
    };
    use crate::audio::session::{
        CaptureSource, CaptureTrackDescriptor, SessionId, SessionMode, SessionOrigin, TrackId,
        TrackSource,
    };
    use crate::audio::vad::{VadDecision, VadKind};

    fn frame(
        session_number: u64,
        sequence: u64,
        start_ms: u64,
        duration_ms: u32,
        sample_rate_hz: u32,
    ) -> AudioFrame {
        AudioFrame {
            session_id: session_id(session_number),
            track_id: track_id(),
            sequence,
            sample_rate_hz,
            channels: 1,
            start_ms,
            duration_ms,
            sample_count: 320,
        }
    }

    fn session_id(value: u64) -> SessionId {
        SessionId::new(format!("s-{value}")).unwrap()
    }

    fn track_id() -> TrackId {
        TrackId::new("mic-1").unwrap()
    }

    fn track_descriptor() -> CaptureTrackDescriptor {
        CaptureTrackDescriptor::from_selector(
            track_id(),
            TrackSource::Captured {
                source: CaptureSource::Microphone,
            },
            "install-id",
            "0:Built-in Microphone",
        )
    }

    fn chunk_context<'a>(
        owner_namespace: &'a crate::audio::session::OwnerNamespace,
        track: &'a CaptureTrackDescriptor,
        encoded_audio: &'a [u8],
    ) -> ChunkBuildContext<'a> {
        ChunkBuildContext {
            owner_namespace,
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::LiveCapture,
            track,
            route: AudioRoute::ServerBatch,
            audio_artifact_id: "artifact-1",
            encoded_audio,
        }
    }

    fn chunk_builder(
        session_number: u64,
        purpose: AudioPurpose,
    ) -> AudioChunkEnvelopeBuilder<'static> {
        let owner_namespace = Box::leak(Box::new(
            crate::audio::session::OwnerNamespace::local("legacy-window").unwrap(),
        ));
        let track = Box::leak(Box::new(track_descriptor()));
        let encoded_audio = Box::leak(Box::new([1_u8, 2, 3]));
        AudioChunkEnvelopeBuilder::new(
            session_id(session_number),
            chunk_context(owner_namespace, track, encoded_audio),
            purpose,
            AudioCodec::PcmS16Le,
        )
    }

    #[test]
    fn builder_rejects_cross_session_cross_track_and_sequence_regression() {
        let owner_namespace = crate::audio::session::OwnerNamespace::local("install-1").unwrap();
        let track = track_descriptor();
        let encoded_audio = [1_u8, 2, 3];

        let mut cross_session = AudioChunkEnvelopeBuilder::new(
            session_id(7),
            chunk_context(&owner_namespace, &track, &encoded_audio),
            AudioPurpose::CaptureEnvelope,
            AudioCodec::PcmS16Le,
        );
        assert!(cross_session.push(frame(8, 1, 0, 20, 16_000)).is_err());

        let mut cross_track = AudioChunkEnvelopeBuilder::new(
            session_id(7),
            chunk_context(&owner_namespace, &track, &encoded_audio),
            AudioPurpose::CaptureEnvelope,
            AudioCodec::PcmS16Le,
        );
        let mut wrong_track = frame(7, 1, 0, 20, 16_000);
        wrong_track.track_id = TrackId::new("mic-2").unwrap();
        assert!(cross_track.push(wrong_track).is_err());

        let mut regression = AudioChunkEnvelopeBuilder::new(
            session_id(7),
            chunk_context(&owner_namespace, &track, &encoded_audio),
            AudioPurpose::CaptureEnvelope,
            AudioCodec::PcmS16Le,
        );
        regression.push(frame(7, 2, 20, 20, 16_000)).unwrap();
        assert!(regression.push(frame(7, 1, 0, 20, 16_000)).is_err());
    }

    #[test]
    fn builder_rejects_impossible_or_overlapping_frame_timing() {
        let owner_namespace = crate::audio::session::OwnerNamespace::local("install-1").unwrap();
        let track = track_descriptor();
        let encoded_audio = [1_u8, 2, 3];

        let mut impossible = AudioChunkEnvelopeBuilder::new(
            session_id(7),
            chunk_context(&owner_namespace, &track, &encoded_audio),
            AudioPurpose::CaptureEnvelope,
            AudioCodec::PcmS16Le,
        );
        assert!(impossible.push(frame(7, 1, 0, 0, 16_000)).is_err());

        let mut overlapping = AudioChunkEnvelopeBuilder::new(
            session_id(7),
            chunk_context(&owner_namespace, &track, &encoded_audio),
            AudioPurpose::CaptureEnvelope,
            AudioCodec::PcmS16Le,
        );
        overlapping.push(frame(7, 1, 0, 20, 16_000)).unwrap();
        assert!(overlapping.push(frame(7, 2, 19, 20, 16_000)).is_err());
    }

    #[test]
    fn session_envelope_serializes_with_expected_field_names() {
        let mut builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
        builder.push(frame(55, 2, 40, 20, 16_000)).unwrap();
        builder.push(frame(55, 3, 60, 20, 16_000)).unwrap();
        let descriptor = builder
            .finish(vec![VadSegment {
                start_ms: 40,
                end_ms: 80,
                kind: VadKind::Speech,
                rms: 0.33,
            }])
            .unwrap()
            .capture_descriptor();
        let session = AudioSessionEnvelope {
            session_id: session_id(55),
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::LiveCapture,
            tracks: vec![track_descriptor()],
            started_at_ms: 1_000,
            sample_rate_hz: 16_000,
            chunks: vec![descriptor],
            degraded: true,
        };

        let value = serde_json::to_value(&session).expect("session envelope should serialize");

        assert_eq!(value["sessionId"], "s-55");
        assert_eq!(value["sessionMode"], "dictation");
        assert_eq!(value["sessionOrigin"], "live_capture");
        assert!(value.get("source").is_none());
        assert_eq!(value["startedAtMs"], 1_000);
        assert_eq!(value["sampleRateHz"], 16_000);
        assert_eq!(value["degraded"], true);
        assert!(value["chunks"][0]["chunkId"]
            .as_str()
            .unwrap()
            .starts_with("chunk-v1-"));
        assert!(value["chunks"][0].get("retry").is_none());
        assert_eq!(value["chunks"][0]["contentIdentity"]["byteLength"], 3);
    }

    #[test]
    fn chunk_builder_accepts_contiguous_frames_in_sequence_order() {
        let mut builder = chunk_builder(7, AudioPurpose::CaptureEnvelope);
        builder.push(frame(7, 11, 100, 20, 16_000)).unwrap();
        builder.push(frame(7, 12, 120, 20, 16_000)).unwrap();

        let vad_segments = vec![VadSegment {
            start_ms: 100,
            end_ms: 140,
            kind: VadKind::Speech,
            rms: 0.42,
        }];

        let envelope = builder
            .finish(vad_segments.clone())
            .expect("frames should build an envelope");

        assert_eq!(envelope.session_id, session_id(7));
        assert_eq!(envelope.track_id, track_id());
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
        let builder = chunk_builder(7, AudioPurpose::LocalFallback);

        assert!(builder.finish(Vec::new()).is_err());
    }

    #[test]
    fn chunk_builder_sets_retry_and_idempotency_fields() {
        let mut builder = chunk_builder(7, AudioPurpose::LocalFallback);
        builder.push(frame(7, 11, 100, 20, 16_000)).unwrap();
        builder.push(frame(7, 12, 120, 20, 16_000)).unwrap();

        let envelope = builder
            .finish(vec![VadSegment {
                start_ms: 100,
                end_ms: 140,
                kind: VadKind::Speech,
                rms: 0.42,
            }])
            .expect("frames should build an envelope");

        assert_eq!(envelope.retry.idempotency_key, envelope.chunk_id);
        assert_eq!(envelope.retry.attempt, 1);
        assert_eq!(envelope.retry.max_attempts, 1);
    }

    #[test]
    fn session_builder_collects_chunks_and_marks_degraded() {
        let mut first_chunk_builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
        first_chunk_builder
            .push(frame(55, 4, 80, 20, 16_000))
            .unwrap();
        first_chunk_builder
            .push(frame(55, 5, 100, 20, 16_000))
            .unwrap();
        let first_chunk = first_chunk_builder
            .finish(vec![VadSegment {
                start_ms: 80,
                end_ms: 120,
                kind: VadKind::Silence,
                rms: 0.0,
            }])
            .expect("first chunk should build");

        let mut second_chunk_builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
        second_chunk_builder
            .push(frame(55, 2, 40, 20, 16_000))
            .unwrap();
        second_chunk_builder
            .push(frame(55, 3, 60, 20, 16_000))
            .unwrap();
        let second_chunk = second_chunk_builder
            .finish(vec![VadSegment {
                start_ms: 40,
                end_ms: 80,
                kind: VadKind::Speech,
                rms: 0.33,
            }])
            .expect("second chunk should build");

        let mut session_builder = AudioSessionEnvelopeBuilder::new(
            session_id(55),
            SessionMode::Dictation,
            SessionOrigin::LiveCapture,
            vec![track_descriptor()],
            1_000,
            16_000,
        );
        session_builder.push_chunk(first_chunk);
        session_builder.push_chunk(second_chunk);
        session_builder.mark_degraded();

        let session = session_builder.finish().unwrap();

        assert_eq!(session.session_id, session_id(55));
        assert_eq!(session.session_origin, SessionOrigin::LiveCapture);
        assert_eq!(session.started_at_ms, 1_000);
        assert_eq!(session.sample_rate_hz, 16_000);
        assert!(session.degraded);
        assert_eq!(session.chunks.len(), 2);
        assert_eq!(session.chunks[0].sequence_start, 2);
        assert_eq!(session.chunks[1].sequence_start, 4);
    }

    #[test]
    fn session_builder_rejects_origin_and_track_source_mismatch() {
        let builder = AudioSessionEnvelopeBuilder::new(
            session_id(55),
            SessionMode::Dictation,
            SessionOrigin::ImportedFile,
            vec![track_descriptor()],
            1_000,
            16_000,
        );

        assert!(builder.finish().is_err());
    }

    fn window_config(preserve_silence_markers: bool) -> ChunkWindowConfig {
        ChunkWindowConfig {
            target_window_ms: 40,
            max_window_ms: 80,
            tail_padding_ms: 20,
            preserve_silence_markers,
        }
    }

    fn windows(
        session_id: u64,
        frames: &[AudioFrame],
        vad: &[VadDecision],
        purpose: AudioPurpose,
        codec: AudioCodec,
        config: ChunkWindowConfig,
    ) -> Vec<crate::audio::frame::AudioChunkEnvelope> {
        build_manifest_windows(session_id, frames, vad, purpose, codec, config).unwrap()
    }

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
    fn legacy_live_audio_source_deserializes_as_live_capture() {
        let manifest: AudioSessionEnvelope = serde_json::from_value(serde_json::json!({
            "sessionId": 7,
            "source": "live",
            "startedAtMs": 0,
            "sampleRateHz": 16_000,
            "chunks": [],
            "degraded": false
        }))
        .unwrap();

        assert_eq!(
            manifest.session_origin,
            crate::audio::session::SessionOrigin::LiveCapture
        );
        assert_eq!(manifest.tracks.len(), 1);
        assert!(matches!(
            manifest.tracks[0].source,
            crate::audio::session::TrackSource::Captured { .. }
        ));
    }

    #[test]
    fn legacy_manifest_with_numeric_nested_chunk_session_ids_deserializes() {
        let manifest: AudioSessionEnvelope = serde_json::from_value(serde_json::json!({
            "sessionId": 7,
            "source": "live",
            "startedAtMs": 0,
            "sampleRateHz": 16_000,
            "chunks": [{
                "sessionId": 7,
                "chunkId": "7-1-20",
                "sequenceStart": 1,
                "startMs": 0,
                "durationMs": 20,
                "sampleRateHz": 16_000,
                "codec": "pcm_s16_le",
                "vadSegments": [],
                "purpose": "captureEnvelope",
                "retry": {
                    "idempotencyKey": "7-1-7-1-20",
                    "attempt": 1,
                    "maxAttempts": 1
                }
            }],
            "degraded": false
        }))
        .unwrap();

        assert_eq!(manifest.session_id.as_str(), "legacy-7");
        assert_eq!(manifest.chunks[0].session_id.as_str(), "legacy-7");
        assert_eq!(manifest.chunks[0].track_id.as_str(), "legacy-0");
        assert_eq!(manifest.chunks[0].sequence_end, 1);
    }

    #[test]
    fn manifest_deserialization_rejects_origin_and_track_source_mismatch() {
        let result = serde_json::from_value::<AudioSessionEnvelope>(serde_json::json!({
            "sessionId": "s-imported",
            "sessionMode": "dictation",
            "sessionOrigin": "imported_file",
            "tracks": [{
                "trackId": "mic-1",
                "source": { "kind": "captured", "source": "microphone" },
                "deviceId": "dev-opaque"
            }],
            "startedAtMs": 0,
            "sampleRateHz": 16_000,
            "chunks": [],
            "degraded": false
        }));

        assert!(result.is_err());
    }

    #[test]
    fn manifest_deserialization_rejects_chunk_replay_key_mismatch() {
        let mut builder = chunk_builder(55, AudioPurpose::CaptureEnvelope);
        builder.push(frame(55, 1, 0, 20, 16_000)).unwrap();
        let session = AudioSessionEnvelope {
            session_id: session_id(55),
            session_mode: SessionMode::Dictation,
            session_origin: SessionOrigin::LiveCapture,
            tracks: vec![track_descriptor()],
            started_at_ms: 0,
            sample_rate_hz: 16_000,
            chunks: vec![builder.finish(Vec::new()).unwrap().capture_descriptor()],
            degraded: false,
        };
        let mut value = serde_json::to_value(session).unwrap();
        value["chunks"][0]["replayKey"]["sequenceEnd"] = serde_json::json!(2);

        assert!(serde_json::from_value::<AudioSessionEnvelope>(value).is_err());
    }

    #[test]
    fn manifest_device_reference_is_opaque_and_does_not_contain_the_os_label() {
        let descriptor = crate::audio::session::CaptureTrackDescriptor::from_selector(
            crate::audio::session::TrackId::new("mic-1").unwrap(),
            crate::audio::session::TrackSource::Captured {
                source: crate::audio::session::CaptureSource::Microphone,
            },
            "install-id",
            "0:Built-in Microphone",
        );

        assert!(descriptor.device_id.starts_with("dev-"));
        assert!(!descriptor.device_id.contains("Built-in Microphone"));
    }
}
