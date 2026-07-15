use super::*;

#[test]
fn level_telemetry_overwrites_with_the_latest_value_when_the_consumer_stalls() {
    let (levels, receiver) = level_channel();

    assert!(publish_level(&levels, 0.25));
    assert!(publish_level(&levels, 0.75));
    assert_eq!(receiver.recv().unwrap(), 0.75);
    assert!(publish_level(&levels, 0.5));
}

#[test]
fn level_telemetry_publication_between_readiness_and_take_keeps_consumer_alive() {
    let (levels, receiver) = level_channel();
    let (ready_seen_tx, ready_seen_rx) = mpsc::channel();
    let publication_complete = Arc::new(Barrier::new(2));
    let consumer_publication_complete = Arc::clone(&publication_complete);
    let (received_tx, received_rx) = mpsc::channel();

    assert!(publish_level(&levels, 0.25));
    let consumer = std::thread::spawn(move || {
        let first = receiver
            .recv_with_ready_hook(|| {
                ready_seen_tx.send(()).unwrap();
                consumer_publication_complete.wait();
            })
            .unwrap();
        received_tx.send(first).unwrap();
        received_tx.send(receiver.recv().unwrap()).unwrap();
        assert!(receiver.recv().is_err());
    });

    ready_seen_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(publish_level(&levels, 0.75));
    publication_complete.wait();
    assert_eq!(
        received_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        0.75
    );

    assert!(publish_level(&levels, 0.5));
    assert_eq!(
        received_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        0.5
    );
    drop(levels);
    consumer.join().unwrap();
}

#[test]
fn level_telemetry_has_explicit_producer_closure_and_receiver_cancellation() {
    let (levels, receiver) = level_channel();
    let remaining_producer = levels.clone();
    drop(levels);

    assert!(publish_level(&remaining_producer, 0.4));
    assert_eq!(receiver.recv().unwrap(), 0.4);

    let closed = std::thread::spawn(move || receiver.recv());
    drop(remaining_producer);
    assert!(closed.join().unwrap().is_err());

    let (levels, receiver) = level_channel();
    drop(receiver);
    assert!(!publish_level(&levels, 0.8));
}

#[test]
fn stop_tail_silence_covers_final_silence_window() {
    assert_eq!(stream::silence_samples(Duration::from_millis(1500)), 24_000);
}

#[test]
fn stream_finisher_reports_backed_up_channel() {
    let (samples_tx, _samples_rx) = mpsc::sync_channel(0);
    let finisher = StreamFinisher::new(samples_tx, 1);

    let status = finisher.finish_session();

    assert_eq!(status, StreamFinishStatus::BackedUp);
    assert!(status.should_retire_stream());
    assert!(status.should_report());
}

#[test]
fn stream_finisher_waits_briefly_for_queue_space() {
    let (samples_tx, samples_rx) = mpsc::sync_channel(1);
    samples_tx
        .try_send(StreamMessage::Samples {
            session: 42,
            samples: vec![1.0],
        })
        .unwrap();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        match samples_rx.recv().unwrap() {
            StreamMessage::Samples { session, .. } => assert_eq!(session, 42),
            StreamMessage::Finish { .. } => panic!("expected queued samples first"),
        }
        match samples_rx.recv().unwrap() {
            StreamMessage::Finish { session, done } => {
                assert_eq!(session, 42);
                done.send(StreamFinishStatus::Completed).unwrap();
            }
            StreamMessage::Samples { .. } => panic!("expected finish message"),
        }
    });
    let finisher = StreamFinisher::new(samples_tx, 42);

    let status = finisher.finish_session();

    assert_eq!(status, StreamFinishStatus::Completed);
    assert!(!status.should_retire_stream());
    worker.join().unwrap();
}

#[test]
fn stream_finisher_reports_completed_channel() {
    let (samples_tx, samples_rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || match samples_rx.recv().unwrap() {
        StreamMessage::Finish { session, done } => {
            assert_eq!(session, 42);
            done.send(StreamFinishStatus::Completed).unwrap();
        }
        StreamMessage::Samples { .. } => panic!("expected finish message"),
    });
    let finisher = StreamFinisher::new(samples_tx, 42);

    let status = finisher.finish_session();

    assert_eq!(status, StreamFinishStatus::Completed);
    assert!(!status.should_retire_stream());
    assert!(!status.should_report());
    worker.join().unwrap();
}

#[test]
fn stream_finisher_reports_disconnected_channel() {
    let (samples_tx, samples_rx) = mpsc::sync_channel(1);
    drop(samples_rx);
    let finisher = StreamFinisher::new(samples_tx, 1);

    let status = finisher.finish_session();

    assert_eq!(status, StreamFinishStatus::Disconnected);
    assert!(status.should_retire_stream());
    assert!(status.should_report());
}
