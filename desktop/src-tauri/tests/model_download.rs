use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use yap_desktop_lib::stt::{
    error::SttError,
    model::{download_file, download_file_with_progress},
};

struct StallingDownloadFixture {
    url: String,
    address: SocketAddr,
    server: Option<thread::JoinHandle<()>>,
}

impl StallingDownloadFixture {
    fn two_requests(stall: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            read_http_request(&mut first);
            let stalled = thread::spawn(move || {
                first
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc",
                    )
                    .unwrap();
                first.flush().unwrap();
                thread::sleep(stall);
                let _ = first.write_all(b"def");
                let _ = first.flush();
            });

            let (mut retry, _) = listener.accept().unwrap();
            read_http_request(&mut retry);
            retry
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef",
                )
                .unwrap();
            retry.flush().unwrap();
            stalled.join().unwrap();
        });

        Self {
            url: format!("http://{address}/model.bin"),
            address,
            server: Some(server),
        }
    }
}

impl Drop for StallingDownloadFixture {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            if !server.is_finished() {
                if let Ok(mut stream) = TcpStream::connect(self.address) {
                    let _ = stream.write_all(
                        b"GET /fixture-shutdown HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    );
                }
            }
            let _ = server.join();
        }
    }
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

fn read_http_request(stream: &mut TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0u8; 512];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }
}

fn owned_partial_files(dest: &Path) -> Vec<PathBuf> {
    let Some(parent) = dest.parent() else {
        return Vec::new();
    };
    let Some(file_name) = dest.file_name().and_then(|name| name.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{file_name}.");
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
fn cancel_interrupts_a_stalled_body_and_allows_immediate_retry() {
    let fixture = StallingDownloadFixture::two_requests(Duration::from_millis(900));
    let dir = TestDir::new("yap-download-stalled-cancel");
    let dest = dir.path.join("model.bin");
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation = Arc::clone(&cancelled);
    let (progress_tx, progress_rx) = mpsc::sync_channel(1);
    let canceller = thread::spawn(move || {
        progress_rx.recv().unwrap();
        thread::sleep(Duration::from_millis(50));
        cancellation.store(true, Ordering::Release);
    });
    let mut progress_tx = Some(progress_tx);
    let started_at = Instant::now();

    let error = download_file_with_progress(
        &fixture.url,
        &dest,
        |_| {
            if let Some(tx) = progress_tx.take() {
                tx.send(()).unwrap();
            }
        },
        || cancelled.load(Ordering::Acquire),
    )
    .unwrap_err();
    let cancel_elapsed = started_at.elapsed();
    canceller.join().unwrap();

    assert_eq!(error, SttError::ModelInstallCancelled);
    assert!(!dest.exists());
    assert!(owned_partial_files(&dest).is_empty());

    download_file(&fixture.url, &dest).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"abcdef");
    assert!(
        cancel_elapsed < Duration::from_millis(400),
        "cancel took {cancel_elapsed:?} while the response body was stalled"
    );
}
