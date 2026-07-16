use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct JournalDelta {
    pub(super) schema_version: u16,
    pub(super) session_id: SessionId,
    pub(super) tracks: Vec<JournalTrack>,
    pub(super) revision_transitions: Vec<RecordingRevisionTransition>,
    pub(super) timeline_gap_start_index: usize,
    pub(super) timeline_gaps: Vec<AudioGap>,
    pub(super) sequence_coverage: Vec<SequenceCoverage>,
    pub(super) gap_start_index: usize,
    pub(super) sequence_gaps: Vec<SequenceGap>,
    pub(super) sequence_gap_overflow: Option<SequenceGapOverflow>,
    pub(super) sink_degraded: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum JournalRecord {
    Header {
        journal: CaptureJournal,
    },
    Delta {
        delta: JournalDelta,
    },
    Overflow {
        session_id: SessionId,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub(super) struct DurableJournalState {
    pub(super) tracks: BTreeSet<String>,
    pub(super) revision_transitions: usize,
    pub(super) timeline_gaps: Vec<AudioGap>,
    pub(super) sequence_coverage: BTreeMap<String, SequenceCoverage>,
    pub(super) sequence_gaps: usize,
}

impl DurableJournalState {
    pub(super) fn from_journal(journal: &CaptureJournal) -> Self {
        debug_assert_eq!(
            journal.track_configurations.len(),
            journal.clock_mappings.len()
        );
        Self {
            tracks: journal.tracks.keys().cloned().collect(),
            revision_transitions: journal.track_configurations.len(),
            timeline_gaps: journal.timeline_gaps.clone(),
            sequence_coverage: journal
                .sequence_coverage
                .iter()
                .map(|coverage| (coverage.track_id.clone(), coverage.clone()))
                .collect(),
            sequence_gaps: journal.sequence_gaps.len(),
        }
    }

    pub(super) fn delta(&self, journal: &CaptureJournal) -> JournalDelta {
        let gap_start_index = self.sequence_gaps.saturating_sub(1);
        let timeline_gap_start_index =
            first_changed_index(&self.timeline_gaps, &journal.timeline_gaps);
        JournalDelta {
            schema_version: CAPTURE_SCHEMA_VERSION,
            session_id: journal.session_id.clone(),
            tracks: journal
                .tracks
                .iter()
                .filter(|(track_id, _)| !self.tracks.contains(*track_id))
                .map(|(_, track)| track.clone())
                .collect(),
            revision_transitions: journal.track_configurations[self.revision_transitions..]
                .iter()
                .cloned()
                .zip(
                    journal.clock_mappings[self.revision_transitions..]
                        .iter()
                        .cloned(),
                )
                .map(
                    |(configuration, clock_mapping)| RecordingRevisionTransition {
                        configuration,
                        clock_mapping,
                    },
                )
                .collect(),
            timeline_gap_start_index,
            timeline_gaps: journal.timeline_gaps[timeline_gap_start_index..].to_vec(),
            sequence_coverage: journal
                .sequence_coverage
                .iter()
                .filter(|coverage| {
                    self.sequence_coverage.get(&coverage.track_id) != Some(*coverage)
                })
                .cloned()
                .collect(),
            gap_start_index,
            sequence_gaps: journal.sequence_gaps[gap_start_index..].to_vec(),
            sequence_gap_overflow: journal.sequence_gap_overflow.clone(),
            sink_degraded: journal.sink_degraded,
        }
    }
}

fn first_changed_index<T: PartialEq>(durable: &[T], current: &[T]) -> usize {
    durable
        .iter()
        .zip(current)
        .position(|(durable, current)| durable != current)
        .unwrap_or_else(|| durable.len().min(current.len()))
}
