use super::*;

#[test]
fn source_positions_convert_to_monotonic_session_time() {
    let track = track_id("mic-1");
    let mut timeline = Timeline::new(session_id());
    timeline
        .configure_track(TrackConfigurationRevision::new(track.clone(), 1, 200, 48_000).unwrap())
        .unwrap();
    timeline
        .map_clock(ClockMappingRevision::new(track.clone(), 1, 9_600, 200).unwrap())
        .unwrap();

    let first = timeline.frame(&track, 12_000, 480, 1).unwrap();
    let second = timeline.frame(&track, 12_480, 480, 1).unwrap();

    assert_eq!((first.start_ms, first.duration_ms), (250, 10));
    assert_eq!((second.start_ms, second.duration_ms), (260, 10));
    assert_eq!(first.end_ms(), second.start_ms);
}

#[test]
fn frame_intervals_are_end_exclusive_without_rounding_drift() {
    let track = track_id("mic-1");
    let mut timeline = configured_timeline(&track, 3);

    let first = timeline.frame(&track, 1, 1, 1).unwrap();
    let second = timeline.frame(&track, 2, 1, 1).unwrap();

    assert_eq!((first.start_ms, first.duration_ms), (333, 333));
    assert_eq!((second.start_ms, second.duration_ms), (666, 334));
    assert_eq!(first.end_ms(), second.start_ms);
}

#[test]
fn source_frame_overlap_is_rejected_even_when_milliseconds_do_not_overlap() {
    let track = track_id("mic-1");
    let mut timeline = configured_timeline(&track, 48_000);

    timeline.frame(&track, 0, 49, 1).unwrap();

    assert_eq!(
        timeline.frame(&track, 48, 48, 1),
        Err(TimelineError::InvalidTiming)
    );
}

#[test]
fn clock_remap_cannot_regress_before_the_checked_source_frame_end() {
    let track = track_id("mic-1");
    let mut timeline = configured_timeline(&track, 48_000);
    timeline.frame(&track, 0, 49, 1).unwrap();

    assert_eq!(
        timeline.map_clock(ClockMappingRevision::new(track, 2, 48, 1).unwrap()),
        Err(TimelineError::InvalidTiming)
    );
}

#[test]
fn frame_sequences_are_owned_per_track() {
    let mic = track_id("mic-1");
    let loopback = track_id("loopback-1");
    let mut timeline = Timeline::new(session_id());
    for track in [&mic, &loopback] {
        timeline
            .configure_track(TrackConfigurationRevision::new(track.clone(), 1, 0, 16_000).unwrap())
            .unwrap();
        timeline
            .map_clock(ClockMappingRevision::new(track.clone(), 1, 0, 0).unwrap())
            .unwrap();
    }

    let mic_first = timeline.frame(&mic, 0, 160, 1).unwrap();
    let loopback_first = timeline.frame(&loopback, 0, 160, 2).unwrap();
    let mic_second = timeline.frame(&mic, 160, 160, 1).unwrap();

    assert_eq!(mic_first.sequence, 0);
    assert_eq!(loopback_first.sequence, 0);
    assert_eq!(mic_second.sequence, 1);
    assert_eq!(loopback_first.sample_count, 320);
}

#[test]
fn contiguous_same_cause_gaps_coalesce() {
    let track = track_id("mic-1");
    let mut timeline = configured_timeline(&track, 16_000);

    timeline
        .gap(
            &track,
            LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 0,
            },
        )
        .unwrap();
    let merged = timeline
        .gap(
            &track,
            LossSnapshot {
                first_source_position_frames: 160,
                dropped_frames: 320,
                cause: GapCause::CallbackPoolExhausted,
                generation: 1,
            },
        )
        .unwrap();

    assert_eq!(merged.source_position_frames, 0);
    assert_eq!(merged.dropped_frames, 480);
    assert_eq!(merged.duration_ms, 30);
    assert_eq!(merged.generation, 1);
}

#[test]
fn same_track_gaps_coalesce_across_other_tracks_and_empty_generations() {
    let mic = track_id("mic-1");
    let loopback = track_id("loopback-1");
    let mut timeline = Timeline::new(session_id());
    for track in [&mic, &loopback] {
        timeline
            .configure_track(TrackConfigurationRevision::new(track.clone(), 1, 0, 16_000).unwrap())
            .unwrap();
        timeline
            .map_clock(ClockMappingRevision::new(track.clone(), 1, 0, 0).unwrap())
            .unwrap();
    }

    timeline
        .gap(
            &mic,
            LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 0,
            },
        )
        .unwrap();
    timeline
        .gap(
            &loopback,
            LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 160,
                cause: GapCause::SinkUnavailable,
                generation: 0,
            },
        )
        .unwrap();
    let merged = timeline
        .gap(
            &mic,
            LossSnapshot {
                first_source_position_frames: 160,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 2,
            },
        )
        .unwrap();

    assert_eq!(merged.dropped_frames, 320);
    assert_eq!(merged.generation, 2);
}

#[test]
fn same_track_configuration_breaks_gap_coalescing() {
    let track = track_id("mic-1");
    let mut timeline = configured_timeline(&track, 16_000);
    let first = timeline
        .gap(
            &track,
            LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 0,
            },
        )
        .unwrap();
    timeline
        .configure_track(TrackConfigurationRevision::new(track.clone(), 2, 10, 16_000).unwrap())
        .unwrap();
    timeline
        .map_clock(ClockMappingRevision::new(track.clone(), 2, 160, 10).unwrap())
        .unwrap();

    let second = timeline
        .gap(
            &track,
            LossSnapshot {
                first_source_position_frames: 160,
                dropped_frames: 160,
                cause: GapCause::CallbackPoolExhausted,
                generation: 1,
            },
        )
        .unwrap();

    assert_eq!(first.dropped_frames, 160);
    assert_eq!(second.source_position_frames, 160);
    assert_eq!(second.dropped_frames, 160);
}

#[test]
fn gap_merge_checks_all_totals_before_mutating_the_existing_event() {
    let track = track_id("mic-1");
    let mut previous = AudioGap {
        session_id: session_id(),
        track_id: track.clone(),
        start_ms: 0,
        duration_ms: 1,
        source_position_frames: 0,
        dropped_frames: u64::MAX,
        cause: GapCause::CallbackPoolExhausted,
        generation: 0,
    };
    let next = AudioGap {
        session_id: session_id(),
        track_id: track,
        start_ms: 1,
        duration_ms: 1,
        source_position_frames: u64::MAX,
        dropped_frames: 1,
        cause: GapCause::CallbackPoolExhausted,
        generation: 1,
    };
    let unchanged = previous.clone();

    assert_eq!(
        super::merge_gap(&mut previous, &next),
        Err(TimelineError::InvalidTiming)
    );
    assert_eq!(previous, unchanged);
}

#[test]
fn non_contiguous_or_different_cause_gaps_do_not_coalesce() {
    let track = track_id("mic-1");
    let mut timeline = configured_timeline(&track, 16_000);
    let gaps = [
        LossSnapshot {
            first_source_position_frames: 0,
            dropped_frames: 160,
            cause: GapCause::CallbackPoolExhausted,
            generation: 0,
        },
        LossSnapshot {
            first_source_position_frames: 320,
            dropped_frames: 160,
            cause: GapCause::CallbackPoolExhausted,
            generation: 1,
        },
        LossSnapshot {
            first_source_position_frames: 480,
            dropped_frames: 160,
            cause: GapCause::OversizedCallback,
            generation: 2,
        },
    ]
    .into_iter()
    .map(|snapshot| timeline.gap(&track, snapshot).unwrap())
    .collect::<Vec<_>>();
    assert_eq!(gaps.len(), 3);
    assert_eq!(gaps[0].dropped_frames, 160);
    assert_eq!(gaps[1].source_position_frames, 320);
    assert_eq!(gaps[2].cause, GapCause::OversizedCallback);
}

#[test]
fn invalid_or_wrapping_source_timing_is_reported_explicitly() {
    let track = track_id("mic-1");
    let mut timeline = configured_timeline(&track, 1);

    assert_eq!(
        timeline.frame(&track, u64::MAX, 1, 1),
        Err(TimelineError::InvalidTiming)
    );
    assert_eq!(
        timeline.gap(
            &track,
            LossSnapshot {
                first_source_position_frames: 0,
                dropped_frames: u64::from(u32::MAX) + 1,
                cause: GapCause::DeviceDiscontinuity,
                generation: 0,
            },
        ),
        Err(TimelineError::InvalidTiming)
    );
}
