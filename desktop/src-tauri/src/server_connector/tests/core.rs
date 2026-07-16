use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

use crate::{
    runtime,
    server_connector::{client, config, ServerCapabilities, ServerConnector},
};

#[test]
fn stale_batch_connection_lease_cannot_commit_after_configuration_changes() {
    let connector = ServerConnector::default();
    connector.synchronize_settings_with(
        &config::ServerSettings {
            schema_version: config::CURRENT_SCHEMA_VERSION,
            enabled: true,
            base_url: Some("http://127.0.0.1:18765".into()),
        },
        |_| {},
    );
    let (generation, _) = connector
        .begin_health_request_with(|_| {})
        .expect("configured connector begins health request");
    connector.accept_health_result_with(
        generation,
        client::HealthCheckResult::Ready {
            api_version: "1".into(),
            capabilities: ServerCapabilities {
                batch_jobs: true,
                live_streaming: false,
                job_status: true,
            },
        },
        |_| {},
        |_, _, _| tauri::async_runtime::spawn(async {}),
    );
    let lease = connector
        .batch_connection_lease()
        .unwrap()
        .expect("ready batch-capable connector yields a lease");
    connector.invalidate();

    let committed = AtomicBool::new(false);
    assert!(connector
        .with_current_batch_lease(&lease, || {
            committed.store(true, Ordering::SeqCst);
        })
        .is_err());
    assert!(!committed.load(Ordering::SeqCst));
}

#[test]
fn settings_load_cannot_run_ahead_of_the_connector_save_lock() {
    let connector = Arc::new(ServerConnector::default());
    let save_guard = connector.inner.lock().unwrap();
    let (load_started_tx, load_started_rx) = mpsc::channel();
    let waiting_connector = Arc::clone(&connector);
    let waiter = std::thread::spawn(move || {
        waiting_connector
            .with_loaded_settings(
                || {
                    load_started_tx.send(()).unwrap();
                    Ok(config::ServerSettings::default())
                },
                |_, _| (),
            )
            .unwrap();
    });

    assert!(load_started_rx
        .recv_timeout(Duration::from_millis(50))
        .is_err());
    drop(save_guard);
    load_started_rx.recv().unwrap();
    waiter.join().unwrap();
}

#[test]
fn delayed_health_response_cannot_mutate_a_new_settings_generation() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let (request_started_tx, request_started_rx) = mpsc::channel();
    let (release_response_tx, release_response_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 1024];
        let read = stream.read(&mut request).unwrap();
        assert!(read > 0);
        request_started_tx.send(()).unwrap();
        release_response_rx.recv().unwrap();
        let body = br#"{"service":"yap-server","status":"ok","apiVersion":"1","auth":"not_configured","capabilities":{"batchJobs":true,"liveStreaming":true,"jobStatus":true}}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    });

    let connector = Arc::new(ServerConnector::default());
    {
        let mut inner = connector.inner.lock().unwrap();
        inner.apply_server_settings(0, true, Some(base_url.clone()));
        assert!(inner.begin_health_request(0, 10));
    }
    let request_connector = Arc::clone(&connector);
    let request = std::thread::spawn(move || {
        tauri::async_runtime::block_on(client::check_health(
            &request_connector.client,
            &base_url,
            false,
        ))
    });

    request_started_rx.recv().unwrap();
    assert_eq!(connector.invalidate(), 1);
    release_response_tx.send(()).unwrap();
    let result = request.join().unwrap();
    server.join().unwrap();

    let mut inner = connector.inner.lock().unwrap();
    assert!(inner
        .finish_health_request(0, result, 20, |_| Duration::ZERO)
        .is_none());
    assert_eq!(
        inner.snapshot().state,
        runtime::state::ServerConnectorState::NotSet
    );
    assert_eq!(inner.snapshot().capabilities, ServerCapabilities::default());
}

#[test]
fn settings_changes_advance_the_connector_generation() {
    let connector = ServerConnector::default();

    assert_eq!(connector.current(), 0);
    assert_eq!(connector.invalidate(), 1);
    assert_eq!(connector.current(), 1);
}

#[test]
fn server_settings_save_has_one_end_to_end_owner() {
    let connector = ServerConnector::default();

    let first = connector.begin_settings_save().unwrap();
    assert_eq!(
        connector.begin_settings_save().unwrap_err(),
        "A server settings update is already active."
    );

    drop(first);
    assert!(connector.begin_settings_save().is_ok());
}
