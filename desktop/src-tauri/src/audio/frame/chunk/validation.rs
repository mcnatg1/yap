use crate::audio::session::{SessionId, SessionMode, SessionOrigin, TrackId, TrackSource};

use super::super::sample::{AudioFrame, AudioGap, ManifestError};
use super::{
    chunk_id_from_replay_key, AudioCodec, AudioPurpose, AudioRoute, CaptureChunkDescriptor,
    ChunkReplayKey, ContentIdentity, VadSegment, CHUNK_SCHEMA_VERSION,
};

pub(super) fn validate_frames(
    session_id: &SessionId,
    track_id: &TrackId,
    frames: &[AudioFrame],
    gaps: &[AudioGap],
) -> Result<(), ManifestError> {
    let first = frames.first().ok_or(ManifestError::EmptyFrames)?;
    validate_gaps(session_id, track_id, frames, gaps)?;
    let mut previous: Option<&AudioFrame> = None;
    for frame in frames {
        if frame.session_id != *session_id {
            return Err(ManifestError::SessionMismatch);
        }
        if frame.track_id != *track_id {
            return Err(ManifestError::TrackMismatch);
        }
        let frame_end_ms = frame.checked_end_ms()?;
        if frame.sample_rate_hz != first.sample_rate_hz {
            return Err(ManifestError::MixedSampleRates);
        }
        if let Some(previous) = previous {
            let previous_end_ms = previous.checked_end_ms()?;
            if frame.sequence <= previous.sequence {
                return Err(ManifestError::SequenceDiscontinuity);
            }
            if frame.start_ms < previous_end_ms {
                return Err(ManifestError::OverlappingFrameTiming);
            }
            let sequence_is_contiguous = frame.sequence == previous.sequence.saturating_add(1);
            let timing_is_contiguous = frame.start_ms == previous_end_ms;
            if (!sequence_is_contiguous || !timing_is_contiguous)
                && !gaps
                    .iter()
                    .any(|gap| gap.covers(session_id, track_id, previous_end_ms, frame.start_ms))
            {
                return Err(if !sequence_is_contiguous {
                    ManifestError::SequenceDiscontinuity
                } else {
                    ManifestError::TimingDiscontinuity
                });
            }
        }
        let _ = frame_end_ms;
        previous = Some(frame);
    }
    Ok(())
}

pub(super) fn validate_vad_segments(
    chunk_start_ms: u64,
    chunk_end_ms: u64,
    vad_segments: &[VadSegment],
) -> Result<(), ManifestError> {
    if vad_segments.iter().any(|segment| {
        segment.end_ms <= segment.start_ms
            || segment.start_ms < chunk_start_ms
            || segment.end_ms > chunk_end_ms
    }) {
        return Err(ManifestError::InvalidVadTiming);
    }
    Ok(())
}

fn validate_gaps(
    session_id: &SessionId,
    track_id: &TrackId,
    frames: &[AudioFrame],
    gaps: &[AudioGap],
) -> Result<(), ManifestError> {
    if gaps.iter().any(|gap| {
        gap.session_id != *session_id
            || gap.track_id != *track_id
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
            || gap.end_ms().is_none()
            || frames.iter().any(|frame| {
                let frame_end_ms = frame.checked_end_ms().unwrap_or(u64::MAX);
                gap.start_ms < frame_end_ms
                    && gap
                        .end_ms()
                        .is_some_and(|gap_end_ms| gap_end_ms > frame.start_ms)
            })
    }) {
        return Err(ManifestError::InvalidGapTiming);
    }
    Ok(())
}

pub(crate) fn validate_route(
    origin: SessionOrigin,
    route: AudioRoute,
    purpose: AudioPurpose,
) -> Result<(), ManifestError> {
    if origin == SessionOrigin::ImportedFile
        && (route == AudioRoute::LocalFallback || purpose == AudioPurpose::LocalFallback)
    {
        return Err(ManifestError::InvalidRouteForOrigin);
    }
    Ok(())
}

pub(crate) fn track_source_matches_origin(origin: SessionOrigin, source: &TrackSource) -> bool {
    matches!(
        (origin, source),
        (SessionOrigin::LiveCapture, TrackSource::Captured { .. })
            | (SessionOrigin::ImportedFile, TrackSource::Imported { .. })
    )
}

pub(crate) fn validate_current_descriptor(
    descriptor: &CaptureChunkDescriptor,
) -> Result<(), ManifestError> {
    if descriptor.replay_key.schema_version != CHUNK_SCHEMA_VERSION
        || !descriptor.content_identity.is_valid_sha256()
        || descriptor.content_identity.byte_length == 0
        || descriptor.replay_key.session_id != descriptor.session_id
        || descriptor.replay_key.track_id != descriptor.track_id
        || descriptor.replay_key.sequence_start != descriptor.sequence_start
        || descriptor.replay_key.sequence_end != descriptor.sequence_end
        || descriptor.chunk_id != chunk_id_from_replay_key(&descriptor.replay_key)
        || descriptor.audio_artifact_id.is_empty()
        || descriptor.sequence_end < descriptor.sequence_start
        || descriptor.duration_ms == 0
        || descriptor.sample_rate_hz == 0
        || !track_source_matches_origin(descriptor.session_origin, &descriptor.track_source)
    {
        return Err(ManifestError::SessionTrackReferenceMismatch);
    }
    validate_route(
        descriptor.session_origin,
        descriptor.route,
        descriptor.purpose,
    )?;
    let chunk_end_ms = descriptor
        .start_ms
        .checked_add(u64::from(descriptor.duration_ms))
        .ok_or(ManifestError::DurationOverflow)?;
    validate_vad_segments(descriptor.start_ms, chunk_end_ms, &descriptor.vad_segments)?;
    let mut internal_gaps = Vec::new();
    for gap in &descriptor.gaps {
        let gap_end_ms = gap.end_ms().ok_or(ManifestError::InvalidGapTiming)?;
        let is_internal = gap.start_ms >= descriptor.start_ms && gap_end_ms <= chunk_end_ms;
        let is_preceding = gap_end_ms == descriptor.start_ms;
        if gap.session_id != descriptor.session_id
            || gap.track_id != descriptor.track_id
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
            || (!is_internal && !is_preceding)
        {
            return Err(ManifestError::InvalidGapTiming);
        }
        if is_internal {
            internal_gaps.push((gap.start_ms, gap_end_ms));
        }
    }
    validate_internal_gap_union(descriptor, &internal_gaps)?;
    Ok(())
}

fn validate_internal_gap_union(
    descriptor: &CaptureChunkDescriptor,
    internal_gaps: &[(u64, u64)],
) -> Result<(), ManifestError> {
    let mut gaps = internal_gaps.to_vec();
    gaps.sort_unstable();

    let mut union_duration = 0_u64;
    let mut current: Option<(u64, u64)> = None;
    for (start_ms, end_ms) in gaps {
        match current {
            Some((_, current_end_ms)) if start_ms < current_end_ms => {
                return Err(ManifestError::InvalidGapTiming);
            }
            Some((current_start_ms, current_end_ms)) if start_ms == current_end_ms => {
                current = Some((current_start_ms, end_ms));
            }
            Some((current_start_ms, current_end_ms)) => {
                union_duration = union_duration
                    .checked_add(current_end_ms - current_start_ms)
                    .ok_or(ManifestError::DurationOverflow)?;
                current = Some((start_ms, end_ms));
            }
            None => current = Some((start_ms, end_ms)),
        }
    }
    if let Some((start_ms, end_ms)) = current {
        union_duration = union_duration
            .checked_add(end_ms - start_ms)
            .ok_or(ManifestError::DurationOverflow)?;
    }
    if union_duration >= u64::from(descriptor.duration_ms) {
        return Err(ManifestError::InvalidGapTiming);
    }
    if descriptor.vad_segments.iter().any(|segment| {
        internal_gaps
            .iter()
            .any(|(start_ms, end_ms)| segment.start_ms < *end_ms && segment.end_ms > *start_ms)
    }) {
        return Err(ManifestError::InvalidGapTiming);
    }
    Ok(())
}

impl<'de> serde::Deserialize<'de> for CaptureChunkDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SchemaOneDescriptor {
            replay_key: ChunkReplayKey,
            content_identity: ContentIdentity,
            session_mode: SessionMode,
            session_origin: SessionOrigin,
            track_source: TrackSource,
            route: AudioRoute,
            audio_artifact_id: String,
            session_id: SessionId,
            track_id: TrackId,
            chunk_id: String,
            sequence_start: u64,
            sequence_end: u64,
            start_ms: u64,
            duration_ms: u32,
            sample_rate_hz: u32,
            codec: AudioCodec,
            vad_segments: Vec<VadSegment>,
            gaps: Vec<AudioGap>,
            purpose: AudioPurpose,
        }

        let schema_one = SchemaOneDescriptor::deserialize(deserializer)?;

        let descriptor = Self {
            replay_key: schema_one.replay_key,
            content_identity: schema_one.content_identity,
            session_mode: schema_one.session_mode,
            session_origin: schema_one.session_origin,
            track_source: schema_one.track_source,
            route: schema_one.route,
            audio_artifact_id: schema_one.audio_artifact_id,
            session_id: schema_one.session_id,
            track_id: schema_one.track_id,
            chunk_id: schema_one.chunk_id,
            sequence_start: schema_one.sequence_start,
            sequence_end: schema_one.sequence_end,
            start_ms: schema_one.start_ms,
            duration_ms: schema_one.duration_ms,
            sample_rate_hz: schema_one.sample_rate_hz,
            codec: schema_one.codec,
            vad_segments: schema_one.vad_segments,
            gaps: schema_one.gaps,
            purpose: schema_one.purpose,
        };
        validate_current_descriptor(&descriptor).map_err(serde::de::Error::custom)?;
        Ok(descriptor)
    }
}
