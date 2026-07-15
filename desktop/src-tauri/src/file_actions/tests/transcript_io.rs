use super::*;

#[test]
fn bounded_transcript_io_keeps_capacity_owned_until_timed_out_work_finishes() {
    let limiter = Arc::new(Semaphore::new(1));
    let first = tauri::async_runtime::block_on(run_bounded_transcript_io(
        Arc::clone(&limiter),
        Duration::from_millis(10),
        "Test read",
        || {
            std::thread::sleep(Duration::from_millis(100));
            Ok("late".into())
        },
    ));
    assert!(first.unwrap_err().contains("timed out"));
    assert_eq!(limiter.available_permits(), 0);

    let second_ran = Arc::new(AtomicBool::new(false));
    let second_ran_in_work = Arc::clone(&second_ran);
    let second = tauri::async_runtime::block_on(run_bounded_transcript_io(
        Arc::clone(&limiter),
        Duration::from_millis(10),
        "Test read",
        move || {
            second_ran_in_work.store(true, Ordering::SeqCst);
            Ok("unexpected".into())
        },
    ));
    assert!(second.unwrap_err().contains("filesystem capacity"));
    assert!(!second_ran.load(Ordering::SeqCst));

    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while limiter.available_permits() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(limiter.available_permits(), 1);
}

#[test]
fn bounded_transcript_io_returns_successful_work() {
    let result = tauri::async_runtime::block_on(run_bounded_transcript_io(
        Arc::new(Semaphore::new(1)),
        Duration::from_secs(1),
        "Test read",
        || Ok("ready".into()),
    ));

    assert_eq!(result.unwrap(), "ready");
}
