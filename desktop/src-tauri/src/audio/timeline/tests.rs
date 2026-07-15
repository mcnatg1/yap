use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use super::{
    merge_gap, ClockMappingRevision, LossAccumulator, LossSnapshot, Timeline, TimelineError,
    TryDrain, LOSS_RUN_CAPACITY, REGISTRATION_TICKET_CAPACITY,
};
use crate::audio::frame::{AudioGap, GapCause, TrackConfigurationRevision};
use crate::audio::session::{SessionId, TrackId};

fn session_id() -> SessionId {
    SessionId::new("s-timeline-test").unwrap()
}

fn track_id(value: &str) -> TrackId {
    TrackId::new(value).unwrap()
}

#[test]
fn clock_mapping_json_rejects_zero_revision() {
    let json = r#"{
        "trackId":"microphone",
        "revision":0,
        "sourcePositionFrames":0,
        "sessionTimeMs":0
    }"#;

    assert!(serde_json::from_str::<ClockMappingRevision>(json).is_err());
}

fn configured_timeline(track: &TrackId, sample_rate_hz: u32) -> Timeline {
    let mut timeline = Timeline::new(session_id());
    timeline
        .configure_track(
            TrackConfigurationRevision::new(track.clone(), 1, 0, sample_rate_hz).unwrap(),
        )
        .unwrap();
    timeline
        .map_clock(ClockMappingRevision::new(track.clone(), 1, 0, 0).unwrap())
        .unwrap();
    timeline
}

fn concurrently_record<const N: usize>(
    records: [(u64, u64, GapCause); N],
) -> Result<Option<LossSnapshot>, TimelineError> {
    let losses = Arc::new(LossAccumulator::new());
    let start = Arc::new(Barrier::new(N + 1));
    let writers = records.map(|(position, dropped, cause)| {
        let losses = Arc::clone(&losses);
        let start = Arc::clone(&start);
        thread::spawn(move || {
            start.wait();
            losses.record(position, dropped, cause);
        })
    });

    start.wait();
    for writer in writers {
        writer.join().unwrap();
    }
    losses.drain()
}

mod concurrency;
mod loss_invariants;
mod timeline_semantics;
