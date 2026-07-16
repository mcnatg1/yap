use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CaptureSidecar {
    pub(super) schema_version: u16,
    pub(super) session_id: SessionId,
    pub(super) audio_file: String,
    pub(super) audio_sha256: String,
    pub(super) audio_bytes: u64,
    pub(super) tracks: Vec<JournalTrack>,
    pub(super) track_configurations: Vec<TrackConfigurationRevision>,
    pub(super) clock_mappings: Vec<ClockMappingRevision>,
    pub(super) timeline_gaps: Vec<AudioGap>,
    pub(super) sequence_coverage: Vec<SequenceCoverage>,
    pub(super) sequence_gaps: Vec<SequenceGap>,
    #[serde(default)]
    pub(super) sequence_gap_overflow: Option<SequenceGapOverflow>,
    pub(super) sink_degraded: bool,
    pub(super) directory_sync_supported: bool,
    #[serde(default)]
    pub(super) session_metadata: Option<SessionMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PartialCaptureSidecar {
    pub(super) schema_version: u16,
    pub(super) session_id: SessionId,
    pub(super) status: CaptureStatus,
}

impl CaptureSidecar {
    pub(super) fn validate(&self, manifest: &CaptureCommitManifest) -> Result<(), String> {
        if self.schema_version != CAPTURE_SCHEMA_VERSION
            || self.session_id != manifest.session_id
            || self.audio_file != manifest.audio_file
            || self.audio_sha256 != manifest.audio_sha256
            || self.audio_bytes != manifest.audio_bytes
        {
            return Err("capture sidecar does not match the commit manifest".into());
        }
        if self.session_metadata != manifest.session_metadata {
            return Err("capture session metadata does not match the commit manifest".into());
        }
        if let Some(metadata) = &self.session_metadata {
            validate_capture_metadata(metadata, &self.session_id)?;
        }
        validate_audio_metadata_presence(self.audio_bytes, &self.tracks)?;
        validate_timeline_control_metadata(
            &self.session_id,
            &self.tracks,
            &self.track_configurations,
            &self.clock_mappings,
            &self.timeline_gaps,
        )?;
        validate_sequence_metadata(
            &self.tracks,
            &self.sequence_coverage,
            &self.sequence_gaps,
            self.sequence_gap_overflow.as_ref(),
            self.sink_degraded,
        )?;
        validate_artifact_name(&self.audio_file)?;
        validate_sha256(&self.audio_sha256)
    }
}

pub(super) fn validate_audio_metadata_presence(
    audio_bytes: u64,
    tracks: &[JournalTrack],
) -> Result<(), String> {
    if audio_bytes > WAV_HEADER_BYTES && tracks.is_empty() {
        return Err("nonempty recording audio has no frame metadata".into());
    }
    Ok(())
}

pub(super) fn validate_timeline_control_metadata<'a>(
    session_id: &SessionId,
    tracks: impl IntoIterator<Item = &'a JournalTrack>,
    track_configurations: &[TrackConfigurationRevision],
    clock_mappings: &[ClockMappingRevision],
    timeline_gaps: &[AudioGap],
) -> Result<(), String> {
    if track_configurations.len() > MAX_TIMELINE_CONTROL_EVENTS
        || clock_mappings.len() > MAX_TIMELINE_CONTROL_EVENTS
        || timeline_gaps.len() > MAX_TIMELINE_CONTROL_EVENTS
    {
        return Err("recording timeline metadata exceeds its fixed bound".into());
    }

    let mut recorded_tracks = BTreeSet::new();
    for track in tracks {
        if track.sample_rate_hz == 0
            || track.channels == 0
            || !recorded_tracks.insert(track.track_id.clone())
        {
            return Err("recording track metadata is invalid".into());
        }
    }

    let mut configurations = BTreeMap::<String, (u32, u64)>::new();
    let mut configuration_revisions = BTreeMap::<(String, u32), (u64, u32)>::new();
    for configuration in track_configurations {
        let track = configuration.track_id.as_str().to_string();
        if configuration.revision == 0 || configuration.sample_rate_hz == 0 {
            return Err("recording track configuration is invalid".into());
        }
        match configurations.get(&track) {
            Some((revision, effective_at_ms))
                if revision.checked_add(1) == Some(configuration.revision)
                    && configuration.effective_at_ms >= *effective_at_ms => {}
            None if configuration.revision == 1 => {}
            _ => return Err("recording track configuration revisions are not contiguous".into()),
        }
        configurations.insert(
            track.clone(),
            (configuration.revision, configuration.effective_at_ms),
        );
        configuration_revisions.insert(
            (track, configuration.revision),
            (configuration.effective_at_ms, configuration.sample_rate_hz),
        );
    }

    let mut mappings = BTreeMap::<String, (u32, u64, u64)>::new();
    let mut revision_clocks = BTreeMap::<String, Vec<(ClockMappingRevision, u32)>>::new();
    for mapping in clock_mappings {
        let track = mapping.track_id.as_str().to_string();
        if !configurations.contains_key(&track) || mapping.revision == 0 {
            return Err("recording clock mapping has no valid track configuration".into());
        }
        let Some((effective_at_ms, sample_rate_hz)) =
            configuration_revisions.get(&(track.clone(), mapping.revision))
        else {
            return Err("recording clock mapping has no matching configuration revision".into());
        };
        if *effective_at_ms != mapping.session_time_ms {
            return Err("recording revision transition timestamp does not match".into());
        }
        match mappings.get(&track) {
            Some((revision, source_position_frames, session_time_ms))
                if revision.checked_add(1) == Some(mapping.revision)
                    && mapping.source_position_frames >= *source_position_frames
                    && mapping.session_time_ms >= *session_time_ms => {}
            None if mapping.revision == 1 => {}
            _ => return Err("recording clock mapping revisions are not contiguous".into()),
        }
        mappings.insert(
            track.clone(),
            (
                mapping.revision,
                mapping.source_position_frames,
                mapping.session_time_ms,
            ),
        );
        revision_clocks
            .entry(track)
            .or_default()
            .push((mapping.clone(), *sample_rate_hz));
    }

    for track in &recorded_tracks {
        if !configurations.contains_key(track) || !mappings.contains_key(track) {
            return Err("recording track has no complete coordinator revision coverage".into());
        }
    }
    for (track, (configuration_revision, _)) in &configurations {
        if mappings.get(track).map(|(revision, _, _)| revision) != Some(configuration_revision) {
            return Err("recording track configuration has no matching clock mapping".into());
        }
    }

    let mut gaps = BTreeMap::<String, (u64, u64, u64, GapCause)>::new();
    for gap in timeline_gaps {
        let track = gap.track_id.as_str().to_string();
        if gap.session_id != *session_id
            || !configurations.contains_key(&track)
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
        {
            return Err("recording timeline gap is invalid".into());
        }
        let end_ms = gap
            .start_ms
            .checked_add(u64::from(gap.duration_ms))
            .ok_or_else(|| "recording timeline gap end overflowed".to_string())?;
        let end_source = gap
            .source_position_frames
            .checked_add(gap.dropped_frames)
            .ok_or_else(|| "recording timeline gap source range overflowed".to_string())?;
        let revisions = revision_clocks
            .get(&track)
            .ok_or_else(|| "recording timeline gap has no clock revisions".to_string())?;
        let revision_index = revisions
            .iter()
            .rposition(|(mapping, _)| mapping.source_position_frames <= gap.source_position_frames)
            .ok_or_else(|| {
                "recording timeline gap precedes its first clock revision".to_string()
            })?;
        let (mapping, sample_rate_hz) = &revisions[revision_index];
        let (expected_start_ms, expected_duration_ms) =
            SessionClock::new(mapping.clone(), *sample_rate_hz)
                .and_then(|clock| clock.interval_ms(gap.source_position_frames, gap.dropped_frames))
                .map_err(|_| "recording timeline gap clock conversion failed".to_string())?;
        if gap.start_ms != expected_start_ms || gap.duration_ms != expected_duration_ms {
            return Err("recording timeline gap does not match its clock revision".into());
        }
        if revisions.get(revision_index + 1).is_some_and(|(next, _)| {
            end_source > next.source_position_frames || end_ms > next.session_time_ms
        }) {
            return Err("recording timeline gap crosses a clock revision".into());
        }
        if let Some((generation, previous_end_ms, previous_end_source, previous_cause)) =
            gaps.get(&track)
        {
            if gap.generation <= *generation
                || gap.start_ms < *previous_end_ms
                || gap.source_position_frames < *previous_end_source
            {
                return Err("recording timeline gaps are not monotonic".into());
            }
            if gap.start_ms == *previous_end_ms
                && gap.source_position_frames == *previous_end_source
                && gap.cause == *previous_cause
            {
                return Err("recording timeline contains an uncoalesced contiguous gap".into());
            }
        }
        gaps.insert(track, (gap.generation, end_ms, end_source, gap.cause));
    }
    Ok(())
}

pub(super) fn validate_sequence_metadata(
    tracks: &[JournalTrack],
    sequence_coverage: &[SequenceCoverage],
    sequence_gaps: &[SequenceGap],
    sequence_gap_overflow: Option<&SequenceGapOverflow>,
    sink_degraded: bool,
) -> Result<(), String> {
    validate_initial_sequence_coverage(sequence_coverage)?;
    let track_ids = tracks
        .iter()
        .map(|track| track.track_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut coverage_by_track = BTreeMap::new();
    for coverage in sequence_coverage {
        if !track_ids.contains(coverage.track_id.as_str())
            || coverage.first_sequence > coverage.last_sequence
            || coverage_by_track
                .insert(coverage.track_id.as_str(), coverage)
                .is_some()
        {
            return Err("recording sequence coverage is invalid".into());
        }
    }
    if coverage_by_track.len() != track_ids.len() {
        return Err("recording track has no sequence coverage".into());
    }

    let mut previous_gap_end = BTreeMap::<&str, u64>::new();
    for gap in sequence_gaps {
        let coverage = coverage_by_track
            .get(gap.track_id.as_str())
            .ok_or_else(|| "recording sequence gap has no coverage".to_string())?;
        let gap_end = gap
            .first_sequence
            .checked_add(gap.dropped_frames)
            .filter(|end| gap.dropped_frames > 0 && *end <= coverage.last_sequence)
            .ok_or_else(|| "recording sequence gap is invalid".to_string())?;
        if gap.first_sequence <= coverage.first_sequence
            || previous_gap_end
                .get(gap.track_id.as_str())
                .is_some_and(|previous_end| gap.first_sequence < *previous_end)
        {
            return Err("recording sequence gaps are not ordered".into());
        }
        previous_gap_end.insert(gap.track_id.as_str(), gap_end);
    }

    if let Some(overflow) = sequence_gap_overflow {
        if overflow.detail_capacity != MAX_SEQUENCE_GAP_DETAILS as u32
            || sequence_gaps.len() != MAX_SEQUENCE_GAP_DETAILS
            || overflow.omitted_gap_count == 0
            || overflow.omitted_dropped_frames == 0
        {
            return Err("recording sequence-gap overflow is invalid".into());
        }
    }
    if (!sequence_gaps.is_empty() || sequence_gap_overflow.is_some()) && !sink_degraded {
        return Err("recording sequence degradation is inconsistent".into());
    }
    if sink_degraded || !sequence_gaps.is_empty() || sequence_gap_overflow.is_some() {
        return Err("degraded recording metadata cannot be complete".into());
    }
    Ok(())
}

pub(super) fn validate_initial_sequence_coverage(
    sequence_coverage: &[SequenceCoverage],
) -> Result<(), String> {
    if sequence_coverage
        .iter()
        .any(|coverage| coverage.first_sequence != 0)
    {
        return Err("recording track sequence must start at zero".into());
    }
    Ok(())
}

pub(super) fn validate_capture_metadata(
    metadata: &SessionMetadata,
    session_id: &SessionId,
) -> Result<(), String> {
    if metadata.session_id != *session_id {
        return Err("capture session metadata does not match the recording session".into());
    }
    Ok(())
}
