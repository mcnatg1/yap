use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(1);

pub(super) struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
    pub(super) fn new(name: &str) -> Self {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "yap-media-owner-{name}-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(super) fn join(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[derive(Debug)]
pub(super) struct RawResponse {
    pub(super) body: Vec<u8>,
    pub(super) headers: String,
    pub(super) status: u16,
}

pub(super) fn request(url: &str, method: &str, range: Option<&str>) -> RawResponse {
    let authority = url
        .strip_prefix("http://")
        .and_then(|value| value.split('/').next())
        .unwrap();
    let path = url.strip_prefix(&format!("http://{authority}")).unwrap();
    let mut stream = TcpStream::connect(authority).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let range = range
        .map(|value| format!("Range: {value}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {authority}\r\n{range}Connection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut bytes = Vec::new();
    if let Err(error) = stream.read_to_end(&mut bytes) {
        // A deliberate `Connection: close` can surface as WSAECONNRESET on
        // Windows after the complete response was delivered. Keep the
        // received bytes; strict assertions still reject an incomplete reply.
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let headers = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let status = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    RawResponse {
        body: bytes[split + 4..].to_vec(),
        headers,
        status,
    }
}

mod admission;
mod http;
mod source;
mod stream;
