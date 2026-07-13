use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use yap_desktop_lib::runtime::state::ServerConnectorState;
use yap_desktop_lib::server_connector::config::{ServerSettings, CURRENT_SCHEMA_VERSION};
use yap_desktop_lib::server_connector::{ServerConnectionSnapshot, ServerConnectorBoundary};

const HEALTHY_BODY: &str = r#"{"service":"yap-server","status":"ok","apiVersion":"1","auth":"not_configured","capabilities":{"batchJobs":true,"liveStreaming":true,"jobStatus":true}}"#;
const AUTH_REQUIRED_BODY: &str = r#"{"service":"yap-server","status":"ok","apiVersion":"1","auth":"required","capabilities":{"batchJobs":false,"liveStreaming":false,"jobStatus":false}}"#;

#[test]
fn healthy_health_contract_projects_ready_capabilities() {
    let server = OneShotServer::respond("200 OK", HEALTHY_BODY);
    let connector = ServerConnectorBoundary::new();

    configure(&connector, true, Some(server.url()));
    let snapshot = refresh(&connector);

    assert_eq!(snapshot.state, ServerConnectorState::Ready);
    assert_eq!(snapshot.api_version.as_deref(), Some("1"));
    assert!(snapshot.capabilities.batch_jobs);
    assert!(snapshot.capabilities.live_streaming);
    assert!(snapshot.capabilities.job_status);
    server.join();
}

#[test]
fn refused_connection_projects_retryable_failure() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let connector = ServerConnectorBoundary::new();

    configure(&connector, true, Some(url));
    let snapshot = refresh(&connector);

    assert_eq!(snapshot.state, ServerConnectorState::Retrying);
    assert_eq!(snapshot.error_code.as_deref(), Some("CONNECTION_FAILED"));
    assert!(snapshot.retry_at_ms.is_some());
    configure(&connector, false, None);
}

#[test]
fn delayed_health_response_projects_timeout() {
    let server = OneShotServer::delayed(Duration::from_millis(3_200));
    let connector = ServerConnectorBoundary::new();

    configure(&connector, true, Some(server.url()));
    let snapshot = refresh(&connector);

    assert_eq!(snapshot.state, ServerConnectorState::Retrying);
    assert_eq!(snapshot.error_code.as_deref(), Some("REQUEST_TIMEOUT"));
    assert!(snapshot.retry_at_ms.is_some());
    configure(&connector, false, None);
    server.join();
}

#[test]
fn malformed_health_response_fails_closed() {
    let server = OneShotServer::respond("200 OK", "{not-json");
    let connector = ServerConnectorBoundary::new();

    configure(&connector, true, Some(server.url()));
    let snapshot = refresh(&connector);

    assert_eq!(snapshot.state, ServerConnectorState::Retrying);
    assert_eq!(
        snapshot.error_code.as_deref(),
        Some("MALFORMED_HEALTH_RESPONSE")
    );
    assert_eq!(snapshot.capabilities, Default::default());
    configure(&connector, false, None);
    server.join();
}

#[test]
fn health_auth_required_projects_sign_in_without_retry() {
    let server = OneShotServer::respond("200 OK", AUTH_REQUIRED_BODY);
    let connector = ServerConnectorBoundary::new();

    configure(&connector, true, Some(server.url()));
    let snapshot = refresh(&connector);

    assert_eq!(snapshot.state, ServerConnectorState::SignInRequired);
    assert_eq!(snapshot.api_version.as_deref(), Some("1"));
    assert_eq!(snapshot.capabilities, Default::default());
    assert_eq!(snapshot.retry_at_ms, None);
    server.join();
}

#[test]
fn disabled_connector_does_not_cross_the_process_boundary() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let connector = ServerConnectorBoundary::new();

    let configured = configure(&connector, false, Some(url));
    let snapshot = refresh(&connector);

    assert_eq!(configured.state, ServerConnectorState::Disabled);
    assert_eq!(snapshot.state, ServerConnectorState::Disabled);
    thread::sleep(Duration::from_millis(100));
    assert!(listener.accept().is_err());
}

#[test]
fn configuration_change_during_request_rejects_the_stale_response() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (request_started_tx, request_started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        request_started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        write_response(&mut stream, "200 OK", HEALTHY_BODY).ok();
    });
    let connector = ServerConnectorBoundary::new();
    configure(&connector, true, Some(url));
    let request_connector = connector.clone();
    let request =
        thread::spawn(move || tauri::async_runtime::block_on(request_connector.refresh()));

    request_started_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let changed = configure(&connector, false, None);
    release_tx.send(()).unwrap();
    let result = request.join().unwrap();

    assert_eq!(changed.state, ServerConnectorState::Disabled);
    assert_eq!(result.state, ServerConnectorState::Disabled);
    assert_eq!(result.capabilities, Default::default());
    assert_eq!(connector.snapshot(), result);
    server.join().unwrap();
}

#[test]
fn disabling_connector_cancels_the_armed_retry() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (second_request_tx, second_request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        read_request(&mut first);
        write_response(&mut first, "500 Internal Server Error", "failure").unwrap();
        drop(first);

        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_millis(1_500);
        let mut second_request = false;
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((_stream, _)) => {
                    second_request = true;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("retry listener failed: {error}"),
            }
        }
        second_request_tx.send(second_request).unwrap();
    });
    let connector = ServerConnectorBoundary::new();

    configure(&connector, true, Some(url));
    let failed = refresh(&connector);
    assert_eq!(failed.state, ServerConnectorState::Retrying);
    assert!(failed.retry_at_ms.is_some());

    let disabled = configure(&connector, false, None);
    assert_eq!(disabled.state, ServerConnectorState::Disabled);
    assert!(!second_request_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap());
    assert_eq!(connector.snapshot().state, ServerConnectorState::Disabled);
    server.join().unwrap();
}

#[test]
fn python_health_process_matches_the_rust_connector_contract_when_provided() {
    let Ok(url) = std::env::var("YAP_TEST_SERVER_URL") else {
        return;
    };
    let connector = ServerConnectorBoundary::new();

    configure(&connector, true, Some(url));
    let snapshot = refresh(&connector);

    assert_eq!(snapshot.state, ServerConnectorState::Ready);
    assert_eq!(snapshot.api_version.as_deref(), Some("1"));
    assert_eq!(snapshot.capabilities, Default::default());
}

fn configure(
    connector: &ServerConnectorBoundary,
    enabled: bool,
    base_url: Option<String>,
) -> ServerConnectionSnapshot {
    connector.configure(&ServerSettings {
        schema_version: CURRENT_SCHEMA_VERSION,
        enabled,
        base_url,
    })
}

fn refresh(connector: &ServerConnectorBoundary) -> ServerConnectionSnapshot {
    tauri::async_runtime::block_on(connector.refresh())
}

struct OneShotServer {
    address: SocketAddr,
    thread: thread::JoinHandle<()>,
}

impl OneShotServer {
    fn respond(status: &'static str, body: &'static str) -> Self {
        Self::spawn(move |mut stream| {
            read_request(&mut stream);
            write_response(&mut stream, status, body).unwrap();
        })
    }

    fn delayed(delay: Duration) -> Self {
        Self::spawn(move |mut stream| {
            read_request(&mut stream);
            thread::sleep(delay);
            write_response(&mut stream, "200 OK", HEALTHY_BODY).ok();
        })
    }

    fn spawn(handler: impl FnOnce(std::net::TcpStream) + Send + 'static) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handler(stream);
        });
        Self { address, thread }
    }

    fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    fn join(self) {
        self.thread.join().unwrap();
    }
}

fn read_request(stream: &mut std::net::TcpStream) {
    let mut request = [0_u8; 2048];
    let read = stream.read(&mut request).unwrap();
    assert!(read > 0);
    assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /v1/health "));
}

fn write_response(
    stream: &mut std::net::TcpStream,
    status: &str,
    body: &str,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
