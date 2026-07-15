use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JournalTrack {
    pub(super) track_id: String,
    pub(super) sample_rate_hz: u32,
    pub(super) channels: u16,
    pub(super) first_start_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SequenceCoverage {
    pub(super) track_id: String,
    pub(super) first_sequence: u64,
    pub(super) last_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SequenceGap {
    pub(super) track_id: String,
    pub(super) first_sequence: u64,
    pub(super) dropped_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SequenceGapOverflow {
    pub(super) detail_capacity: u32,
    pub(super) omitted_gap_count: u64,
    pub(super) omitted_dropped_frames: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CaptureJournal {
    pub(super) schema_version: u16,
    pub(super) session_id: SessionId,
    pub(super) tracks: BTreeMap<String, JournalTrack>,
    pub(super) track_configurations: Vec<TrackConfigurationRevision>,
    pub(super) clock_mappings: Vec<ClockMappingRevision>,
    pub(super) timeline_gaps: Vec<AudioGap>,
    pub(super) sequence_coverage: Vec<SequenceCoverage>,
    pub(super) sequence_gaps: Vec<SequenceGap>,
    #[serde(default)]
    pub(super) sequence_gap_overflow: Option<SequenceGapOverflow>,
    pub(super) sink_degraded: bool,
}

impl CaptureJournal {
    pub(super) fn new(session_id: SessionId) -> Self {
        Self {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id,
            tracks: BTreeMap::new(),
            track_configurations: Vec::new(),
            clock_mappings: Vec::new(),
            timeline_gaps: Vec::new(),
            sequence_coverage: Vec::new(),
            sequence_gaps: Vec::new(),
            sequence_gap_overflow: None,
            sink_degraded: false,
        }
    }

    pub(super) fn observe_frame(
        &mut self,
        track_id: &str,
        sample_rate_hz: u32,
        channels: u16,
        sequence: u64,
        start_ms: u64,
    ) {
        self.tracks
            .entry(track_id.to_string())
            .or_insert(JournalTrack {
                track_id: track_id.to_string(),
                sample_rate_hz,
                channels,
                first_start_ms: start_ms,
            });
        let coverage_index = self
            .sequence_coverage
            .iter()
            .position(|coverage| coverage.track_id == track_id);
        match coverage_index {
            Some(index)
                if sequence
                    == self.sequence_coverage[index]
                        .last_sequence
                        .saturating_add(1) =>
            {
                self.sequence_coverage[index].last_sequence = sequence;
            }
            Some(index) if sequence > self.sequence_coverage[index].last_sequence => {
                let first_missing = self.sequence_coverage[index]
                    .last_sequence
                    .saturating_add(1);
                self.record_gap(track_id, first_missing, sequence);
                self.sequence_coverage[index].last_sequence = sequence;
            }
            Some(_) => self.sink_degraded = true,
            None => self.sequence_coverage.push(SequenceCoverage {
                track_id: track_id.to_string(),
                first_sequence: sequence,
                last_sequence: sequence,
            }),
        }
    }

    pub(super) fn observe_revision_transition(
        &mut self,
        transition: RecordingRevisionTransition,
    ) -> Result<(), String> {
        let configuration = &transition.configuration;
        let mapping = &transition.clock_mapping;
        if self.track_configurations.len() >= MAX_TIMELINE_CONTROL_EVENTS
            || self.clock_mappings.len() >= MAX_TIMELINE_CONTROL_EVENTS
        {
            return Err("recording revision-transition metadata limit reached".into());
        }
        if configuration.track_id != mapping.track_id
            || configuration.revision != mapping.revision
            || configuration.effective_at_ms != mapping.session_time_ms
        {
            return Err("recording revision transition is inconsistent".into());
        }
        let previous = self
            .track_configurations
            .iter()
            .rev()
            .find(|previous| previous.track_id == configuration.track_id);
        match previous {
            Some(previous)
                if previous.revision.checked_add(1) == Some(configuration.revision)
                    && configuration.effective_at_ms >= previous.effective_at_ms
                    && configuration.sample_rate_hz > 0 => {}
            None if configuration.revision == 1 && configuration.sample_rate_hz > 0 => {}
            _ => return Err("recording track configuration is not monotonic".into()),
        }
        let previous = self
            .clock_mappings
            .iter()
            .rev()
            .find(|previous| previous.track_id == mapping.track_id);
        match previous {
            Some(previous)
                if previous.revision.checked_add(1) == Some(mapping.revision)
                    && mapping.source_position_frames >= previous.source_position_frames
                    && mapping.session_time_ms >= previous.session_time_ms => {}
            None if mapping.revision == 1 => {}
            _ => return Err("recording clock mapping is not monotonic".into()),
        }
        self.track_configurations.push(transition.configuration);
        self.clock_mappings.push(transition.clock_mapping);
        Ok(())
    }

    pub(super) fn observe_gap(&mut self, gap: AudioGap) -> Result<(), String> {
        if gap.session_id != self.session_id
            || gap.duration_ms == 0
            || gap.dropped_frames == 0
            || !self
                .track_configurations
                .iter()
                .any(|configuration| configuration.track_id == gap.track_id)
            || gap.end_ms().is_none()
            || gap
                .source_position_frames
                .checked_add(gap.dropped_frames)
                .is_none()
        {
            return Err("recording timeline gap is invalid".into());
        }
        if let Some(index) = self.timeline_gaps.iter().position(|previous| {
            previous.session_id == gap.session_id
                && previous.track_id == gap.track_id
                && previous.cause == gap.cause
                && previous.start_ms == gap.start_ms
                && previous.source_position_frames == gap.source_position_frames
        }) {
            let previous = &self.timeline_gaps[index];
            if gap.generation <= previous.generation
                || gap.duration_ms < previous.duration_ms
                || gap.dropped_frames < previous.dropped_frames
                || self.timeline_gaps[index + 1..]
                    .iter()
                    .any(|later| later.track_id == gap.track_id)
            {
                return Err("recording timeline gap replacement regressed".into());
            }
            self.timeline_gaps[index] = gap;
            return Ok(());
        }
        if self.timeline_gaps.len() >= MAX_TIMELINE_CONTROL_EVENTS {
            return Err("recording timeline-gap metadata limit reached".into());
        }
        if let Some(previous) = self
            .timeline_gaps
            .iter()
            .rev()
            .find(|previous| previous.track_id == gap.track_id)
        {
            let previous_end_ms = previous
                .end_ms()
                .ok_or_else(|| "recording timeline gap end overflowed".to_string())?;
            let previous_end_source = previous
                .source_position_frames
                .checked_add(previous.dropped_frames)
                .ok_or_else(|| "recording timeline gap source range overflowed".to_string())?;
            if gap.generation <= previous.generation
                || gap.start_ms < previous_end_ms
                || gap.source_position_frames < previous_end_source
            {
                return Err("recording timeline gap is not monotonic".into());
            }
            if gap.cause == previous.cause
                && gap.start_ms == previous_end_ms
                && gap.source_position_frames == previous_end_source
            {
                return Err("recording timeline gap was not coalesced".into());
            }
        }
        self.timeline_gaps.push(gap);
        Ok(())
    }

    pub(super) fn record_gap(&mut self, track_id: &str, first_sequence: u64, next_sequence: u64) {
        let dropped_frames = next_sequence.saturating_sub(first_sequence);
        if dropped_frames == 0 {
            return;
        }
        self.sink_degraded = true;
        if let Some(previous) = self.sequence_gaps.last_mut() {
            if previous.track_id == track_id
                && previous.first_sequence.checked_add(previous.dropped_frames)
                    == Some(first_sequence)
            {
                previous.dropped_frames = previous.dropped_frames.saturating_add(dropped_frames);
                return;
            }
        }
        if self.sequence_gaps.len() < MAX_SEQUENCE_GAP_DETAILS {
            self.sequence_gaps.push(SequenceGap {
                track_id: track_id.to_string(),
                first_sequence,
                dropped_frames,
            });
            return;
        }
        let overflow = self
            .sequence_gap_overflow
            .get_or_insert(SequenceGapOverflow {
                detail_capacity: MAX_SEQUENCE_GAP_DETAILS as u32,
                omitted_gap_count: 0,
                omitted_dropped_frames: 0,
            });
        overflow.omitted_gap_count = overflow.omitted_gap_count.saturating_add(1);
        overflow.omitted_dropped_frames = overflow
            .omitted_dropped_frames
            .saturating_add(dropped_frames);
    }

    #[cfg(test)]
    pub(super) fn serialized_len(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |value| value.len())
    }
}
