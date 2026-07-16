use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

use super::{
    admission::{token_from_request_path, MediaOwnerInner},
    http::{
        read_request, select_range, write_empty_response, write_request_error, write_response_head,
        RangeError, SOCKET_TIMEOUT,
    },
    source::FileRangeReader,
};

const MEDIA_SERVER_WORKERS: usize = 4;
const MAX_PENDING_CONNECTIONS: usize = 16;
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(100);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(10);
pub(super) const STREAM_BUFFER_BYTES: usize = 64 * 1024;

pub(super) struct MediaServer {
    accept_thread: Option<JoinHandle<()>>,
    authority: String,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl MediaServer {
    pub(super) fn start(owner: Arc<MediaOwnerInner>) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("Failed to bind media server: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("Failed to configure media server: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("Failed to inspect media server: {error}"))?;
        let authority = address.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_CONNECTIONS);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(MEDIA_SERVER_WORKERS);

        for index in 0..MEDIA_SERVER_WORKERS {
            let receiver = Arc::clone(&receiver);
            let owner = Arc::clone(&owner);
            let authority = authority.clone();
            let stop = Arc::clone(&stop);
            workers.push(
                std::thread::Builder::new()
                    .name(format!("yap-media-{index}"))
                    .spawn(move || media_worker(receiver, owner, authority, stop))
                    .map_err(|error| format!("Failed to start media worker: {error}"))?,
            );
        }

        let accept_stop = Arc::clone(&stop);
        let accept_thread = std::thread::Builder::new()
            .name("yap-media-accept".into())
            .spawn(move || {
                while !accept_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                            if let Err(mpsc::TrySendError::Full(rejected)) = sender.try_send(stream)
                            {
                                stream = rejected;
                                let _ = write_empty_response(
                                    &mut stream,
                                    503,
                                    "Service Unavailable",
                                    &[],
                                );
                            }
                        }
                        Ok((_stream, _peer)) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(ACCEPT_RETRY_DELAY);
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| format!("Failed to start media server: {error}"))?;

        Ok(Self {
            accept_thread: Some(accept_thread),
            authority,
            stop,
            workers,
        })
    }

    pub(super) fn authority(&self) -> &str {
        &self.authority
    }
}

impl Drop for MediaServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(&self.authority);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn media_worker(
    receiver: Arc<Mutex<mpsc::Receiver<TcpStream>>>,
    owner: Arc<MediaOwnerInner>,
    authority: String,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        let received = match receiver.lock() {
            Ok(receiver) => receiver.recv_timeout(WORKER_POLL_INTERVAL),
            Err(_) => return,
        };
        match received {
            Ok(stream) => handle_connection(stream, &owner, &authority),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn handle_connection(mut stream: TcpStream, owner: &MediaOwnerInner, authority: &str) {
    let _ = stream.set_read_timeout(Some(SOCKET_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SOCKET_TIMEOUT));
    let request = match read_request(&mut stream, authority) {
        Ok(request) => request,
        Err(status) => {
            let _ = write_request_error(&mut stream, status, 0);
            return;
        }
    };
    if request.method != "GET" && request.method != "HEAD" {
        let _ = write_empty_response(
            &mut stream,
            405,
            "Method Not Allowed",
            &[("Allow", "GET, HEAD".into())],
        );
        return;
    }
    let Some(token) = token_from_request_path(&request.path) else {
        let _ = write_empty_response(&mut stream, 404, "Not Found", &[]);
        return;
    };
    let Some(entry) = owner.admission(token) else {
        let _ = write_empty_response(&mut stream, 404, "Not Found", &[]);
        return;
    };
    if entry.revoked().load(Ordering::Acquire) {
        let _ = write_empty_response(&mut stream, 404, "Not Found", &[]);
        return;
    }
    if !entry.source_is_unchanged() {
        owner.revoke_if_current(token, &entry);
        let _ = write_empty_response(&mut stream, 410, "Gone", &[]);
        return;
    }
    if !owner.is_current(token, &entry) {
        let _ = write_empty_response(&mut stream, 404, "Not Found", &[]);
        return;
    }

    let total_length = entry.byte_length();
    let selected = match select_range(request.range.as_deref(), total_length) {
        Ok(selected) => selected,
        Err(RangeError::Malformed) => {
            let _ = write_request_error(&mut stream, 400, total_length);
            return;
        }
        Err(RangeError::Multiple | RangeError::Unsatisfiable) => {
            let _ = write_request_error(&mut stream, 416, total_length);
            return;
        }
    };
    let status = if selected.partial { 206 } else { 200 };
    let reason = if selected.partial {
        "Partial Content"
    } else {
        "OK"
    };
    let mut headers = vec![
        ("Accept-Ranges", "bytes".into()),
        ("Content-Length", selected.length.to_string()),
        ("Content-Type", entry.mime().into()),
    ];
    if selected.partial {
        headers.push((
            "Content-Range",
            format!(
                "bytes {}-{}/{}",
                selected.start,
                selected.start + selected.length - 1,
                total_length
            ),
        ));
    }
    if write_response_head(&mut stream, status, reason, &headers).is_err()
        || request.method == "HEAD"
        || selected.length == 0
    {
        return;
    }

    let mut reader = FileRangeReader::new(entry.source_file(), selected.start);
    match stream_exact_range(
        &mut reader,
        &mut stream,
        selected.length,
        entry.revoked().as_ref(),
    ) {
        Ok(StreamOutcome::Complete) => {
            if !entry.source_is_unchanged() {
                owner.revoke_if_current(token, &entry);
            }
        }
        Ok(StreamOutcome::Revoked) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            owner.revoke_if_current(token, &entry);
        }
        Err(_) => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StreamOutcome {
    Complete,
    Revoked,
}

pub(super) fn stream_exact_range<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    mut remaining: u64,
    revoked: &AtomicBool,
) -> std::io::Result<StreamOutcome> {
    let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES];
    while remaining > 0 {
        if revoked.load(Ordering::Acquire) {
            return Ok(StreamOutcome::Revoked);
        }
        let requested = usize::try_from(remaining.min(STREAM_BUFFER_BYTES as u64))
            .expect("bounded media read fits usize");
        let read = reader.read(&mut buffer[..requested])?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "media ended before the admitted range",
            ));
        }
        if revoked.load(Ordering::Acquire) {
            return Ok(StreamOutcome::Revoked);
        }
        writer.write_all(&buffer[..read])?;
        remaining -= read as u64;
    }
    Ok(StreamOutcome::Complete)
}
