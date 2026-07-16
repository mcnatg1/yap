use crate::audio::frame::{CaptureChunkDescriptor, ManifestError, TrackConfigurationRevision};
use crate::audio::session::{CaptureTrackDescriptor, SessionId, SessionMode, SessionOrigin};

pub(super) fn validate_track_sources(
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
pub(super) fn validate_chunk_references(
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
