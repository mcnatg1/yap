use super::*;

#[test]
fn shared_warmup_is_cancellable_reentrant_and_never_duplicates_the_model() {
    let warmup = Arc::new(SharedWarmup::<usize>::new());
    let loads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicBool::new(false));
    let (loader_entered_tx, loader_entered_rx) = mpsc::channel();
    let (release_loader_tx, release_loader_rx) = mpsc::channel();
    let loader_warmup = Arc::clone(&warmup);
    let loader_loads = Arc::clone(&loads);

    assert!(warmup
        .request("test-live-warmup", move || {
            loader_loads.fetch_add(1, Ordering::SeqCst);
            assert!(loader_warmup.is_loading_for_test());
            loader_entered_tx.send(()).unwrap();
            release_loader_rx.recv().unwrap();
            Ok(7)
        })
        .unwrap());
    loader_entered_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert!(!warmup
        .request("duplicate-live-warmup", || panic!("duplicate model load"))
        .unwrap());

    let waiter_warmup = Arc::clone(&warmup);
    let waiter_cancelled = Arc::clone(&cancelled);
    let (waiter_done_tx, waiter_done_rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let result = waiter_warmup
            .wait_cancellable(|| waiter_cancelled.load(Ordering::Acquire))
            .unwrap();
        waiter_done_tx.send(result.is_none()).unwrap();
    });
    cancelled.store(true, Ordering::Release);
    warmup.cancel_loading();
    assert!(waiter_done_rx.recv_timeout(Duration::from_secs(1)).unwrap());
    waiter.join().unwrap();

    assert!(!warmup
        .request("adopt-live-warmup", || panic!("duplicate model load"))
        .unwrap());
    release_loader_tx.send(()).unwrap();
    let lease = warmup
        .wait_cancellable(|| false)
        .unwrap()
        .expect("adopted warmup must publish its model");
    assert_eq!(lease.commit(), 7);
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    warmup.release_in_use();
}

#[test]
fn clearing_idle_warmup_drops_a_ready_model() {
    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let warmup = SharedWarmup::new();
    *warmup.state.lock().unwrap() = SharedWarmupState::Ready(DropSignal(Arc::clone(&dropped)));

    warmup.clear_idle().unwrap();

    assert!(dropped.load(Ordering::Acquire));
    assert!(matches!(
        *warmup.state.lock().unwrap(),
        SharedWarmupState::Empty
    ));
}

#[test]
fn clearing_idle_warmup_cancels_and_waits_for_a_loading_model() {
    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    let warmup = Arc::new(SharedWarmup::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker_dropped = Arc::clone(&dropped);
    warmup
        .request("clear-idle-loading-model", move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(DropSignal(worker_dropped))
        })
        .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let clearing = Arc::clone(&warmup);
    let (cleared_tx, cleared_rx) = mpsc::channel();
    let clearer = std::thread::spawn(move || {
        let result = clearing.clear_idle();
        cleared_tx.send(result).unwrap();
    });
    assert!(cleared_rx.recv_timeout(Duration::from_millis(50)).is_err());
    release_tx.send(()).unwrap();

    assert_eq!(
        cleared_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        Ok(())
    );
    clearer.join().unwrap();
    assert!(dropped.load(Ordering::Acquire));
    assert!(matches!(
        *warmup.state.lock().unwrap(),
        SharedWarmupState::Empty
    ));
}

#[test]
fn model_mutation_lease_invalidates_a_start_queued_behind_it() {
    let runtime = LiveRuntime::new();
    let mutation = runtime.begin_model_mutation().unwrap();
    let intent = runtime.capture_start_intent();
    let queued_runtime = runtime.clone();
    let queued_gate = Arc::clone(&runtime.transition);
    let (waiting_tx, waiting_rx) = mpsc::channel();
    let queued = std::thread::spawn(move || {
        let _operation = queued_gate.begin_start_with_wait_hook(|| waiting_tx.send(()).unwrap());
        queued_runtime.start_intent_is_current(intent)
    });
    waiting_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    drop(mutation);

    assert!(!queued.join().unwrap());
    assert!(!runtime.model_mutation_active.load(Ordering::Acquire));
}

#[test]
fn model_mutation_lease_rejects_new_start_work_without_waiting() {
    let runtime = LiveRuntime::new();
    let _mutation = runtime.begin_model_mutation().unwrap();
    let intent = runtime.capture_start_intent();
    let ran = Arc::new(AtomicBool::new(false));
    let ran_in_start = Arc::clone(&ran);

    let result = runtime.run_start_lifecycle(intent, move || {
        ran_in_start.store(true, Ordering::Release);
    });

    assert!(result.is_none());
    assert!(!ran.load(Ordering::Acquire));
}

#[test]
fn concurrent_recording_finalizers_share_one_cached_result_and_one_worker_finalization() {
    let runtime = LiveRuntime::new();
    let directory =
        std::env::temp_dir().join(format!("yap-runtime-finalize-race-{}", std::process::id()));
    std::fs::remove_dir_all(&directory).ok();
    let session_id = SessionId::new("runtime-finalize-race").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
    let (recording, finalization_count) =
        RecordingSinkHandle::spawn_with_finalization_counter_for_test(
            directory.clone(),
            session_id.clone(),
            sink,
            receiver,
        );
    runtime.inner.lock().unwrap().recording = Some(recording);
    let barrier = Arc::new(Barrier::new(3));
    let left_runtime = runtime.clone();
    let left_barrier = Arc::clone(&barrier);
    let left = std::thread::spawn(move || {
        left_barrier.wait();
        left_runtime.finalize_recording().unwrap()
    });
    let right_runtime = runtime.clone();
    let right_barrier = Arc::clone(&barrier);
    let right = std::thread::spawn(move || {
        right_barrier.wait();
        right_runtime.finalize_recording().unwrap()
    });

    barrier.wait();
    let left = left.join().unwrap();
    let right = right.join().unwrap();

    assert_eq!(left, right);
    assert_eq!(
        finalization_count.load(Ordering::SeqCst),
        1,
        "only one caller may close, join, and publish the recording"
    );
    assert!(directory
        .join(format!("live-{session_id}.commit.json"))
        .is_file());
    assert_eq!(runtime.finalize_recording().unwrap(), left);
    std::fs::remove_dir_all(directory).ok();
}

#[test]
fn racing_stops_share_one_live_stop_result_and_one_recording_finalization() {
    let runtime = LiveRuntime::new();
    let directory =
        std::env::temp_dir().join(format!("yap-runtime-stop-race-{}", std::process::id()));
    std::fs::remove_dir_all(&directory).ok();
    let session_id = SessionId::new("runtime-stop-race").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
    let (recording, finalization_count) =
        RecordingSinkHandle::spawn_with_finalization_counter_for_test(
            directory.clone(),
            session_id,
            sink,
            receiver,
        );
    runtime.inner.lock().unwrap().recording = Some(recording);
    let barrier = Arc::new(Barrier::new(3));
    let left_runtime = runtime.clone();
    let left_barrier = Arc::clone(&barrier);
    let left = std::thread::spawn(move || {
        left_barrier.wait();
        left_runtime.stop()
    });
    let right_runtime = runtime.clone();
    let right_barrier = Arc::clone(&barrier);
    let right = std::thread::spawn(move || {
        right_barrier.wait();
        right_runtime.stop()
    });

    barrier.wait();
    let left = left.join().unwrap();
    let right = right.join().unwrap();

    assert_eq!(left, right);
    assert_eq!(
        finalization_count.load(Ordering::SeqCst),
        1,
        "racing stops must share the finalization lease"
    );
    assert_eq!(runtime.stop(), left);
    std::fs::remove_dir_all(directory).ok();
}

#[test]
fn poisoned_runtime_inner_publishes_one_terminal_error_and_wakes_waiters() {
    let runtime = LiveRuntime::new();
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let poison_runtime = runtime.clone();
    let poisoner = std::thread::spawn(move || {
        let _inner = poison_runtime.inner.lock().unwrap();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        panic!("injected live runtime poison");
    });
    locked_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let first_runtime = runtime.clone();
    let first = std::thread::spawn(move || first_runtime.finalize_recording());
    wait_for_recording_finalizing(&runtime);
    let second_runtime = runtime.clone();
    let second = std::thread::spawn(move || second_runtime.finalize_recording());

    release_tx.send(()).unwrap();
    assert!(poisoner.join().is_err());
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    let repeated = runtime.finalize_recording();

    assert_eq!(first, second);
    assert_eq!(first, repeated);
    assert_eq!(first.unwrap_err(), "live runtime became unavailable");
}

#[test]
fn direct_stop_then_start_rejects_unconsumed_recording_until_finalized() {
    let runtime = LiveRuntime::new();
    let session_id = SessionId::new("s-direct-restart").unwrap();
    runtime.install_unavailable_recording_for_test(session_id.clone());

    assert_eq!(
        runtime.ensure_recording_ready_to_start(),
        Err("Previous live recording must be finalized before starting again.".into())
    );
    assert_eq!(
        runtime.finalize_recording(),
        Err("recording worker is unavailable".into())
    );
    assert_eq!(
        runtime.recording_finalization_failure(),
        Some((session_id, "recording worker is unavailable".into()))
    );
    assert!(runtime.ensure_recording_ready_to_start().is_ok());
}

#[test]
fn direct_stop_then_successful_finalize_allows_the_next_start() {
    let runtime = LiveRuntime::new();
    let directory =
        std::env::temp_dir().join(format!("yap-runtime-direct-restart-{}", std::process::id()));
    std::fs::remove_dir_all(&directory).ok();
    let session_id = SessionId::new("s-direct-restart-success").unwrap();
    let (sink, receiver) = bounded_sink(SinkKind::Recording, 1);
    runtime.inner.lock().unwrap().recording = Some(RecordingSinkHandle::spawn(
        directory.clone(),
        session_id,
        sink,
        receiver,
    ));

    assert!(runtime.ensure_recording_ready_to_start().is_err());
    assert_eq!(
        runtime.finalize_recording().unwrap().unwrap().status,
        crate::audio::recording::CaptureStatus::Complete
    );
    assert!(runtime.ensure_recording_ready_to_start().is_ok());
    std::fs::remove_dir_all(directory).ok();
}
