use super::super::*;
use crate::live::settings::LiveSettings;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc, Barrier,
};

#[test]
fn concurrent_start_claims_capture_once_and_leaves_listening_stoppable() {
    let state = Arc::new(LiveSessionState::new(LiveSettings::default()));
    let race_point = Arc::new(Barrier::new(3));
    let capture_starts = Arc::new(AtomicUsize::new(0));
    let (results_tx, results_rx) = mpsc::channel();

    let workers = (0..2)
        .map(|_| {
            let state = Arc::clone(&state);
            let race_point = Arc::clone(&race_point);
            let capture_starts = Arc::clone(&capture_starts);
            let results_tx = results_tx.clone();
            std::thread::spawn(move || {
                // Both action callers complete preflight before racing for the state claim.
                race_point.wait();
                let claimed = state
                    .try_begin_local_start(LiveCaptureMode::Toggle, None, Some("Default".into()))
                    .is_some();
                if claimed {
                    capture_starts.fetch_add(1, Ordering::SeqCst);
                    state.try_begin_listening_from_armed().unwrap();
                }
                results_tx.send(claimed).unwrap();
            })
        })
        .collect::<Vec<_>>();
    drop(results_tx);

    race_point.wait();
    let claims = results_rx.iter().collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }

    assert_eq!(claims.iter().filter(|claimed| **claimed).count(), 1);
    assert_eq!(capture_starts.load(Ordering::SeqCst), 1);
    assert_eq!(state.snapshot().status, LiveSessionStatus::Listening);

    assert!(state.try_begin_saving(true).is_some());
    assert_eq!(state.finish_saving().status, LiveSessionStatus::Idle);
}
