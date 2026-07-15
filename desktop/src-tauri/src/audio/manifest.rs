use crate::audio::frame::{
    AudioChunkEnvelope, AudioCodec, AudioFrame, AudioPurpose, AudioRoute, CaptureChunkDescriptor,
    ChunkBuildContext, ChunkReplayKey, ContentIdentity, ManifestError, PreparedFrame,
    TrackConfigurationRevision, VadSegment,
};
use crate::audio::session::{
    CaptureSource, CaptureTrackDescriptor, OwnerNamespace, SessionId, SessionMode, SessionOrigin,
    TrackSource,
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

pub const MANIFEST_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSessionEnvelope {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub tracks: Vec<CaptureTrackDescriptor>,
    pub track_configuration_revisions: Vec<TrackConfigurationRevision>,
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
    track_configuration_revisions: Vec<TrackConfigurationRevision>,
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
            track_configuration_revisions: Vec::new(),
            chunks: Vec::new(),
            degraded: false,
        }
    }

    pub fn push_chunk(&mut self, chunk: AudioChunkEnvelope) {
        self.chunks.push(chunk.capture_descriptor());
    }

    pub fn push_track_configuration_revision(&mut self, revision: TrackConfigurationRevision) {
        self.track_configuration_revisions.push(revision);
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
            self.sample_rate_hz,
            &self.track_configuration_revisions,
            &self.chunks,
        )?;
        let degraded = self.degraded || self.chunks.iter().any(|chunk| !chunk.gaps.is_empty());

        Ok(AudioSessionEnvelope {
            schema_version: MANIFEST_SCHEMA_VERSION,
            session_id: self.session_id,
            session_mode: self.session_mode,
            session_origin: self.session_origin,
            tracks: self.tracks,
            track_configuration_revisions: self.track_configuration_revisions,
            started_at_ms: self.started_at_ms,
            sample_rate_hz: self.sample_rate_hz,
            chunks: self.chunks,
            degraded,
        })
    }
}

impl<'de> serde::Deserialize<'de> for AudioSessionEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        fn field_is_present<'de, D>(deserializer: D) -> Result<bool, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            <serde::de::IgnoredAny as serde::Deserialize>::deserialize(deserializer)?;
            Ok(true)
        }

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SchemaOneEnvelope {
            schema_version: u16,
            session_id: SessionId,
            session_mode: SessionMode,
            session_origin: SessionOrigin,
            tracks: Vec<CaptureTrackDescriptor>,
            track_configuration_revisions: Vec<TrackConfigurationRevision>,
            #[serde(rename = "source", default, deserialize_with = "field_is_present")]
            source_present: bool,
            started_at_ms: u64,
            sample_rate_hz: u32,
            chunks: Vec<CaptureChunkDescriptor>,
            degraded: bool,
        }

        let schema_one = SchemaOneEnvelope::deserialize(deserializer)?;
        if schema_one.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(serde::de::Error::custom(
                "unsupported manifest schema version",
            ));
        }
        if schema_one.source_present {
            return Err(serde::de::Error::custom(
                "schema 1 manifests cannot use the source field",
            ));
        }

        let manifest = Self {
            schema_version: schema_one.schema_version,
            session_id: schema_one.session_id,
            session_mode: schema_one.session_mode,
            session_origin: schema_one.session_origin,
            tracks: schema_one.tracks,
            track_configuration_revisions: schema_one.track_configuration_revisions,
            started_at_ms: schema_one.started_at_ms,
            sample_rate_hz: schema_one.sample_rate_hz,
            chunks: schema_one.chunks,
            degraded: schema_one.degraded,
        };

        validate_track_sources(manifest.session_origin, &manifest.tracks)
            .map_err(serde::de::Error::custom)?;
        validate_chunk_references(
            &manifest.session_id,
            manifest.session_mode,
            manifest.session_origin,
            &manifest.tracks,
            manifest.sample_rate_hz,
            &manifest.track_configuration_revisions,
            &manifest.chunks,
        )
        .map_err(serde::de::Error::custom)?;
        if manifest.chunks.iter().any(|chunk| !chunk.gaps.is_empty()) && !manifest.degraded {
            return Err(serde::de::Error::custom(
                "manifests containing gaps must be degraded",
            ));
        }

        Ok(manifest)
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
        crate::audio::frame::track_source_matches_origin(session_origin, &track.source)
    }) {
        return Ok(());
    }
    Err(format!(
        "{session_origin:?} sessions must contain only {expected} tracks"
    ))
}

#[allow(clippy::too_many_arguments)]
fn validate_chunk_references(
    session_id: &SessionId,
    session_mode: SessionMode,
    session_origin: SessionOrigin,
    tracks: &[CaptureTrackDescriptor],
    sample_rate_hz: u32,
    track_configuration_revisions: &[TrackConfigurationRevision],
    chunks: &[CaptureChunkDescriptor],
) -> Result<(), ManifestError> {
    if sample_rate_hz == 0 {
        return Err(ManifestError::InvalidConfigurationRevision);
    }
    validate_track_configuration_revisions(tracks, track_configuration_revisions)?;
    for chunk in chunks {
        crate::audio::frame::validate_current_descriptor(chunk)?;
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
        for gap in &chunk.gaps {
            let gap_end_ms = gap.end_ms().ok_or(ManifestError::InvalidGapTiming)?;
            if chunks.iter().any(|retained| {
                retained.track_id == gap.track_id
                    && retained_audio_intervals(retained)
                        .iter()
                        .any(|(start_ms, end_ms)| *start_ms < gap_end_ms && *end_ms > gap.start_ms)
            }) {
                return Err(ManifestError::InvalidGapTiming);
            }
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
                            && gap.start_ms == previous_end_ms
                            && gap
                                .end_ms()
                                .is_some_and(|gap_end| gap_end == chunk.start_ms)
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
    validate_cross_chunk_sample_rates(sample_rate_hz, track_configuration_revisions, chunks)
}

fn retained_audio_intervals(chunk: &CaptureChunkDescriptor) -> Vec<(u64, u64)> {
    let Some(chunk_end_ms) = chunk.start_ms.checked_add(u64::from(chunk.duration_ms)) else {
        return Vec::new();
    };
    let mut intervals = vec![(chunk.start_ms, chunk_end_ms)];
    for gap in &chunk.gaps {
        let Some(gap_end_ms) = gap.end_ms() else {
            continue;
        };
        intervals = intervals
            .into_iter()
            .flat_map(|(start_ms, end_ms)| {
                if gap_end_ms <= start_ms || gap.start_ms >= end_ms {
                    vec![(start_ms, end_ms)]
                } else {
                    let mut retained = Vec::with_capacity(2);
                    if start_ms < gap.start_ms {
                        retained.push((start_ms, gap.start_ms));
                    }
                    if gap_end_ms < end_ms {
                        retained.push((gap_end_ms, end_ms));
                    }
                    retained
                }
            })
            .collect();
    }
    intervals
}

fn validate_track_configuration_revisions(
    tracks: &[CaptureTrackDescriptor],
    revisions: &[TrackConfigurationRevision],
) -> Result<(), ManifestError> {
    for track in tracks {
        let mut previous_revision = 0;
        let mut previous_effective_at_ms = 0;
        for revision in revisions
            .iter()
            .filter(|revision| revision.track_id == track.track_id)
        {
            if revision.revision <= previous_revision
                || (previous_revision != 0 && revision.effective_at_ms < previous_effective_at_ms)
                || revision.sample_rate_hz == 0
            {
                return Err(ManifestError::InvalidConfigurationRevision);
            }
            previous_revision = revision.revision;
            previous_effective_at_ms = revision.effective_at_ms;
        }
    }
    if revisions.iter().any(|revision| {
        !tracks
            .iter()
            .any(|track| track.track_id == revision.track_id)
    }) {
        return Err(ManifestError::InvalidConfigurationRevision);
    }
    Ok(())
}

fn validate_cross_chunk_sample_rates(
    baseline_sample_rate_hz: u32,
    revisions: &[TrackConfigurationRevision],
    chunks: &[CaptureChunkDescriptor],
) -> Result<(), ManifestError> {
    for chunk in chunks {
        let configured_sample_rate_hz = revisions
            .iter()
            .filter(|revision| {
                revision.track_id == chunk.track_id && revision.effective_at_ms <= chunk.start_ms
            })
            .max_by_key(|revision| (revision.effective_at_ms, revision.revision))
            .map_or(baseline_sample_rate_hz, |revision| revision.sample_rate_hz);
        if chunk.sample_rate_hz != configured_sample_rate_hz {
            return Err(ManifestError::MissingConversionRevision);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct FrameVadAssignment {
    kind: VadKind,
    rms: f32,
}

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

fn build_chunk(
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

fn build_error_windows(
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

fn has_mixed_session_or_sample_rate(session_id: u64, frames: &[PreparedFrame]) -> bool {
    let expected_sample_rate_hz = frames[0].metadata.sample_rate_hz;
    let expected_session_id =
        SessionId::new(format!("s-{session_id}")).expect("legacy numeric session ID is valid");
    frames.iter().any(|frame| {
        frame.metadata.session_id != expected_session_id
            || frame.metadata.sample_rate_hz != expected_sample_rate_hz
    })
}

fn assign_vad(frame: &PreparedFrame, vad: &[VadDecision]) -> FrameVadAssignment {
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

fn advance_while_kind(assignments: &[FrameVadAssignment], start: usize, kind: VadKind) -> usize {
    let mut end = start + 1;
    while end < assignments.len() && assignments[end].kind == kind {
        end += 1;
    }
    end
}

fn split_windows(frames: &[PreparedFrame], window_ms: u64) -> Vec<(usize, usize)> {
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

struct TailExtension {
    chunk_start: usize,
    speech_chunk_end: usize,
    speech_end_ms: u64,
    assigned_speech_end_ms: u64,
    tail_padding_ms: u32,
    max_window_ms: u64,
}

fn extend_speech_tail(
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

fn max_rms(assignments: &[FrameVadAssignment], kind: VadKind) -> f32 {
    assignments
        .iter()
        .filter(|assignment| assignment.kind == kind)
        .map(|assignment| assignment.rms)
        .fold(0.0_f32, f32::max)
}

#[cfg(test)]
mod tests;
