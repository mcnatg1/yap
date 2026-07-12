use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use yap_desktop_lib::stt::{
    error::SttError,
    model::{download_verified_file, DownloadOperation, DownloadRequest},
};

const TEST_BODY: &[u8] = b"abcdef";
const TEST_SHA256: &str = "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721";
const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

struct TestDir(PathBuf);

impl TestDir {
    fn new(prefix: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

struct OneShotServer {
    url: String,
    worker: Option<thread::JoinHandle<()>>,
}

impl OneShotServer {
    fn respond(raw_response: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(&raw_response).unwrap();
            stream.flush().unwrap();
            let _ = stream.shutdown(Shutdown::Write);
        });
        Self {
            url: format!("http://{address}/model.bin"),
            worker: Some(worker),
        }
    }
}

impl Drop for OneShotServer {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

struct HeaderStallServer {
    url: String,
    request_seen: Receiver<()>,
    connection_closed: Receiver<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl HeaderStallServer {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_seen_tx, request_seen) = mpsc::sync_channel(1);
        let (connection_closed_tx, connection_closed) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(EVENT_TIMEOUT)).unwrap();
            read_request(&mut stream);
            request_seen_tx.send(()).unwrap();
            wait_for_client_close(&mut stream, "stalled response header");
            connection_closed_tx.send(()).unwrap();
        });
        Self {
            url: format!("http://{address}/model.bin"),
            request_seen,
            connection_closed,
            worker: Some(worker),
        }
    }

    fn wait_for_request(&self) {
        self.request_seen.recv_timeout(EVENT_TIMEOUT).unwrap();
    }

    fn wait_for_connection_close(&self) {
        self.connection_closed.recv_timeout(EVENT_TIMEOUT).unwrap();
    }
}

impl Drop for HeaderStallServer {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

struct ResponseStallServer {
    url: String,
    connection_closed: Receiver<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ResponseStallServer {
    fn respond_then_stall(response_prefix: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (connection_closed_tx, connection_closed) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(EVENT_TIMEOUT)).unwrap();
            read_request(&mut stream);
            stream.write_all(response_prefix).unwrap();
            stream.flush().unwrap();
            wait_for_client_close(&mut stream, "stalled response");
            connection_closed_tx.send(()).unwrap();
        });
        Self {
            url: format!("http://{address}/model.bin"),
            connection_closed,
            worker: Some(worker),
        }
    }

    fn wait_for_connection_close(&self) {
        self.connection_closed.recv_timeout(EVENT_TIMEOUT).unwrap();
    }
}

impl Drop for ResponseStallServer {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 512];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).unwrap();
        assert_ne!(read, 0, "client closed before completing request headers");
        request.extend_from_slice(&buffer[..read]);
    }
}

fn wait_for_client_close(stream: &mut TcpStream, phase: &str) {
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                panic!("download future did not close the {phase} connection")
            }
            Err(error) => panic!("{phase} connection read failed: {error}"),
        }
    }
}

fn response_with_length(length: usize, body: &[u8]) -> Vec<u8> {
    let mut response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
            .into_bytes();
    response.extend_from_slice(body);
    response
}

fn response_until_close(body: &[u8]) -> Vec<u8> {
    let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    response.extend_from_slice(body);
    response
}

fn request(url: String, destination: PathBuf) -> DownloadRequest {
    DownloadRequest {
        url,
        destination,
        expected_bytes: TEST_BODY.len() as u64,
        expected_sha256: TEST_SHA256.to_string(),
    }
}

fn operation_temps(destination: &Path, generation: u64) -> Vec<PathBuf> {
    let parent = destination.parent().unwrap();
    let file_name = destination.file_name().unwrap().to_string_lossy();
    let prefix = format!("{file_name}.op-{generation}-");
    std::fs::read_dir(parent)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".part"))
        })
        .collect()
}

#[test]
fn stale_download_temps_are_cleaned_without_touching_the_installed_model() {
    let server = OneShotServer::respond(response_with_length(7, b"abcdefg"));
    let dir = TestDir::new("yap-stale-download-temp");
    let destination = dir.join("model.bin");
    let operation_temp = dir.join("model.bin.op-7-123-456-0.part");
    let legacy_unique_temp = dir.join("model.bin.123.456.0.part");
    let legacy_temp = destination.with_extension("part");
    let unrelated_temp = dir.join("model.bin.notes.part");
    std::fs::write(&destination, b"verified-old").unwrap();
    std::fs::write(&operation_temp, b"stale").unwrap();
    std::fs::write(&legacy_unique_temp, b"stale").unwrap();
    std::fs::write(&legacy_temp, b"stale").unwrap();
    std::fs::write(&unrelated_temp, b"keep").unwrap();

    let error = download_verified_file(
        &request(server.url.clone(), destination.clone()),
        &DownloadOperation::new(40),
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error, SttError::ModelCorrupt);
    assert_eq!(std::fs::read(&destination).unwrap(), b"verified-old");
    assert!(!operation_temp.exists());
    assert!(!legacy_unique_temp.exists());
    assert!(!legacy_temp.exists());
    assert_eq!(std::fs::read(unrelated_temp).unwrap(), b"keep");
}

#[test]
fn request_header_wait_is_cancelled_and_terminated_before_retry() {
    let server = HeaderStallServer::new();
    let dir = TestDir::new("yap-header-cancel");
    let destination = dir.join("model.bin");
    std::fs::write(&destination, b"verified-old").unwrap();
    let operation = DownloadOperation::new(41);
    let worker_operation = operation.clone();
    let worker_request = request(server.url.clone(), destination.clone());
    let worker =
        thread::spawn(move || download_verified_file(&worker_request, &worker_operation, |_| {}));

    server.wait_for_request();
    operation.cancel();
    assert_eq!(worker.join().unwrap(), Err(SttError::ModelInstallCancelled));
    server.wait_for_connection_close();
    assert_eq!(std::fs::read(&destination).unwrap(), b"verified-old");
    assert!(operation_temps(&destination, operation.generation()).is_empty());
    drop(server);

    let retry_server = OneShotServer::respond(response_with_length(TEST_BODY.len(), TEST_BODY));
    let retry = DownloadOperation::new(42);
    download_verified_file(
        &request(retry_server.url.clone(), destination.clone()),
        &retry,
        |_| {},
    )
    .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), TEST_BODY);
}

#[test]
fn response_body_wait_is_cancelled_and_reaped_at_a_progress_barrier() {
    let server = ResponseStallServer::respond_then_stall(
        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc",
    );
    let dir = TestDir::new("yap-body-cancel");
    let destination = dir.join("model.bin");
    std::fs::write(&destination, b"verified-old").unwrap();
    let operation = DownloadOperation::new(43);
    let worker_operation = operation.clone();
    let worker_request = request(server.url.clone(), destination.clone());
    let (progress_tx, progress_rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        download_verified_file(&worker_request, &worker_operation, |progress| {
            if progress.downloaded_bytes == 3 {
                let _ = progress_tx.try_send(());
            }
        })
    });

    progress_rx.recv_timeout(EVENT_TIMEOUT).unwrap();
    operation.cancel();
    assert_eq!(worker.join().unwrap(), Err(SttError::ModelInstallCancelled));
    server.wait_for_connection_close();
    assert_eq!(std::fs::read(&destination).unwrap(), b"verified-old");
    assert!(operation_temps(&destination, operation.generation()).is_empty());
}

#[test]
fn known_content_length_mismatch_is_rejected_before_replacement() {
    let server = ResponseStallServer::respond_then_stall(
        b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n",
    );
    let dir = TestDir::new("yap-header-size");
    let destination = dir.join("model.bin");
    std::fs::write(&destination, b"verified-old").unwrap();

    let error = download_verified_file(
        &request(server.url.clone(), destination.clone()),
        &DownloadOperation::new(51),
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error, SttError::ModelCorrupt);
    server.wait_for_connection_close();
    assert_eq!(std::fs::read(destination).unwrap(), b"verified-old");
}

#[test]
fn unknown_length_overrun_is_rejected_without_replacing_existing_file() {
    let server = OneShotServer::respond(response_until_close(b"abcdefg"));
    let dir = TestDir::new("yap-body-overrun");
    let destination = dir.join("model.bin");
    std::fs::write(&destination, b"verified-old").unwrap();

    let error = download_verified_file(
        &request(server.url.clone(), destination.clone()),
        &DownloadOperation::new(52),
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error, SttError::ModelCorrupt);
    assert_eq!(std::fs::read(destination).unwrap(), b"verified-old");
}

#[test]
fn truncated_eof_is_rejected_without_replacing_existing_file() {
    let server = OneShotServer::respond(response_until_close(b"abc"));
    let dir = TestDir::new("yap-body-truncated");
    let destination = dir.join("model.bin");
    std::fs::write(&destination, b"verified-old").unwrap();

    let error = download_verified_file(
        &request(server.url.clone(), destination.clone()),
        &DownloadOperation::new(53),
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error, SttError::ModelCorrupt);
    assert_eq!(std::fs::read(destination).unwrap(), b"verified-old");
}

#[test]
fn hash_failure_preserves_existing_verified_file_and_cleans_owned_temp() {
    let server = OneShotServer::respond(response_with_length(6, b"abcdeg"));
    let dir = TestDir::new("yap-hash-preserve");
    let destination = dir.join("model.bin");
    std::fs::write(&destination, b"verified-old").unwrap();
    let operation = DownloadOperation::new(54);

    let error = download_verified_file(
        &request(server.url.clone(), destination.clone()),
        &operation,
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error, SttError::ModelCorrupt);
    assert_eq!(std::fs::read(&destination).unwrap(), b"verified-old");
    assert!(operation_temps(&destination, operation.generation()).is_empty());
}

#[test]
fn verified_download_atomically_replaces_existing_file() {
    let server = OneShotServer::respond(response_with_length(TEST_BODY.len(), TEST_BODY));
    let dir = TestDir::new("yap-atomic-replace");
    let destination = dir.join("model.bin");
    std::fs::write(&destination, b"verified-old").unwrap();
    let operation = DownloadOperation::new(55);

    download_verified_file(
        &request(server.url.clone(), destination.clone()),
        &operation,
        |_| {},
    )
    .unwrap();

    assert_eq!(std::fs::read(&destination).unwrap(), TEST_BODY);
    assert!(operation_temps(&destination, operation.generation()).is_empty());
}
