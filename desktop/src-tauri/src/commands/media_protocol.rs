use std::{
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
    time::Duration,
};

// Tauri 2.11 custom protocol responders require a complete Cow<'static, [u8]>
// body. A loopback owner preserves HTTP range semantics without buffering media.
const DEFAULT_MAX_ACTIVE_ADMISSIONS: usize = 1024;
const MEDIA_SERVER_WORKERS: usize = 4;
const MAX_PENDING_CONNECTIONS: usize = 16;
const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;
const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_LENGTH: usize = TOKEN_BYTES * 2;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);
const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(100);
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(10);
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
pub(crate) const STREAM_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaAdmission {
    pub(crate) byte_length: String,
    pub(crate) url: String,
    pub(crate) waveform_eligible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaSourceFingerprint {
    identity: FileIdentity,
    length: u64,
}

pub(crate) struct MediaOwner {
    inner: Arc<MediaOwnerInner>,
    server: Mutex<Option<MediaServer>>,
}

impl Default for MediaOwner {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaOwner {
    pub(crate) fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_ACTIVE_ADMISSIONS)
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(MediaOwnerInner {
                registry: Mutex::new(MediaRegistry {
                    capacity: capacity.max(1),
                    entries: HashMap::new(),
                    order: VecDeque::new(),
                }),
            }),
            server: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_capacity_for_test(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    pub(crate) fn admit(
        &self,
        path: &Path,
        waveform_byte_limit: u64,
    ) -> Result<MediaAdmission, String> {
        let fingerprint = inspect_media_source(path)?;
        self.admit_unchanged(path, &fingerprint, waveform_byte_limit)
    }

    pub(crate) fn admit_unchanged(
        &self,
        path: &Path,
        expected: &MediaSourceFingerprint,
        waveform_byte_limit: u64,
    ) -> Result<MediaAdmission, String> {
        if !path.is_absolute() {
            return Err("Recording playback requires an absolute path.".into());
        }
        let mime = media_mime(path)
            .ok_or_else(|| "Choose a supported audio or video file.".to_string())?;
        let file = open_no_follow(path)
            .map_err(|error| format!("Failed to open recording for playback: {error}"))?;
        let snapshot = file_snapshot(&file)?;
        drop(file);
        if &snapshot != expected {
            return Err("Recording source changed while playback was being authorized.".into());
        }

        let authority = self.ensure_server()?;
        let token = self.inner.insert_admission(MediaEntry {
            identity: snapshot.identity,
            length: snapshot.length,
            mime,
            path: path.to_path_buf(),
            revoked: Arc::new(AtomicBool::new(false)),
        })?;
        let (byte_length, waveform_eligible) =
            admission_metadata(snapshot.length, waveform_byte_limit);
        Ok(MediaAdmission {
            byte_length,
            url: format!("http://{authority}/media/{token}"),
            waveform_eligible,
        })
    }

    pub(crate) fn release(&self, url: &str) -> bool {
        let authority = match self.server.lock() {
            Ok(server) => server.as_ref().map(|server| server.authority.clone()),
            Err(_) => None,
        };
        let Some(authority) = authority else {
            return false;
        };
        let Some(token) = token_from_url(url, &authority) else {
            return false;
        };
        self.inner.revoke(&token)
    }

    fn ensure_server(&self) -> Result<String, String> {
        let mut server = self
            .server
            .lock()
            .map_err(|_| "Media server lock is unavailable.".to_string())?;
        if server.is_none() {
            *server = Some(MediaServer::start(Arc::clone(&self.inner))?);
        }
        Ok(server
            .as_ref()
            .expect("media server was initialized")
            .authority
            .clone())
    }

    #[cfg(test)]
    pub(crate) fn active_admission_count_for_test(&self) -> usize {
        self.inner
            .registry
            .lock()
            .map(|registry| registry.entries.len())
            .unwrap_or(0)
    }
}

pub(crate) fn inspect_media_source(path: &Path) -> Result<MediaSourceFingerprint, String> {
    if !path.is_absolute() {
        return Err("Recording playback requires an absolute path.".into());
    }
    media_mime(path).ok_or_else(|| "Choose a supported audio or video file.".to_string())?;
    let file = open_no_follow(path)
        .map_err(|error| format!("Failed to open recording for playback: {error}"))?;
    file_snapshot(&file)
}

pub(crate) fn open_unchanged_media_source(
    path: &Path,
    expected: &MediaSourceFingerprint,
) -> Result<File, String> {
    if !path.is_absolute() {
        return Err("Recording preprocessing requires an absolute path.".into());
    }
    media_mime(path).ok_or_else(|| "Choose a supported audio or video file.".to_string())?;
    let file = open_no_follow(path)
        .map_err(|error| format!("Failed to open recording for preprocessing: {error}"))?;
    let snapshot = file_snapshot(&file)?;
    if &snapshot != expected {
        return Err("Recording source changed before preprocessing began.".into());
    }
    Ok(file)
}

fn admission_metadata(length: u64, waveform_byte_limit: u64) -> (String, bool) {
    (length.to_string(), length <= waveform_byte_limit)
}

struct MediaOwnerInner {
    registry: Mutex<MediaRegistry>,
}

impl MediaOwnerInner {
    fn insert_admission(&self, entry: MediaEntry) -> Result<String, String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "Media registry lock is unavailable.".to_string())?;
        while registry.entries.len() >= registry.capacity {
            let Some(oldest) = registry.order.pop_front() else {
                break;
            };
            if let Some(entry) = registry.entries.remove(&oldest) {
                entry.revoked.store(true, Ordering::Release);
            }
        }

        for _ in 0..4 {
            let token = random_token()?;
            if registry.entries.contains_key(&token) {
                continue;
            }
            registry.order.push_back(token.clone());
            registry.entries.insert(token.clone(), Arc::new(entry));
            return Ok(token);
        }
        Err("Failed to mint a unique media admission token.".into())
    }

    fn admission(&self, token: &str) -> Option<Arc<MediaEntry>> {
        self.registry.lock().ok()?.entries.get(token).cloned()
    }

    fn is_current(&self, token: &str, expected: &Arc<MediaEntry>) -> bool {
        self.registry
            .lock()
            .ok()
            .and_then(|registry| registry.entries.get(token).cloned())
            .is_some_and(|current| Arc::ptr_eq(&current, expected))
            && !expected.revoked.load(Ordering::Acquire)
    }

    fn revoke(&self, token: &str) -> bool {
        let Ok(mut registry) = self.registry.lock() else {
            return false;
        };
        let Some(entry) = registry.entries.remove(token) else {
            return false;
        };
        entry.revoked.store(true, Ordering::Release);
        registry.order.retain(|candidate| candidate != token);
        true
    }

    fn revoke_if_current(&self, token: &str, expected: &Arc<MediaEntry>) {
        let Ok(mut registry) = self.registry.lock() else {
            expected.revoked.store(true, Ordering::Release);
            return;
        };
        let is_current = registry
            .entries
            .get(token)
            .is_some_and(|current| Arc::ptr_eq(current, expected));
        if is_current {
            registry.entries.remove(token);
            registry.order.retain(|candidate| candidate != token);
        }
        expected.revoked.store(true, Ordering::Release);
    }
}

struct MediaRegistry {
    capacity: usize,
    entries: HashMap<String, Arc<MediaEntry>>,
    order: VecDeque<String>,
}

struct MediaEntry {
    identity: FileIdentity,
    length: u64,
    mime: &'static str,
    path: PathBuf,
    revoked: Arc<AtomicBool>,
}

struct MediaServer {
    accept_thread: Option<JoinHandle<()>>,
    authority: String,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl MediaServer {
    fn start(owner: Arc<MediaOwnerInner>) -> Result<Self, String> {
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
    if entry.revoked.load(Ordering::Acquire) {
        let _ = write_empty_response(&mut stream, 404, "Not Found", &[]);
        return;
    }

    let mut file = match open_admitted_file(&entry) {
        Ok(file) => file,
        Err(AdmissionOpenError::Unavailable) => {
            owner.revoke_if_current(token, &entry);
            let _ = write_empty_response(&mut stream, 410, "Gone", &[]);
            return;
        }
        Err(AdmissionOpenError::Internal) => {
            let _ = write_empty_response(&mut stream, 500, "Internal Server Error", &[]);
            return;
        }
    };
    if !owner.is_current(token, &entry) {
        let _ = write_empty_response(&mut stream, 404, "Not Found", &[]);
        return;
    }

    let selected = match select_range(request.range.as_deref(), entry.length) {
        Ok(selected) => selected,
        Err(RangeError::Malformed) => {
            let _ = write_request_error(&mut stream, 400, entry.length);
            return;
        }
        Err(RangeError::Multiple | RangeError::Unsatisfiable) => {
            let _ = write_request_error(&mut stream, 416, entry.length);
            return;
        }
    };
    if selected.start > 0 && file.seek(SeekFrom::Start(selected.start)).is_err() {
        owner.revoke_if_current(token, &entry);
        let _ = write_empty_response(&mut stream, 410, "Gone", &[]);
        return;
    }

    let status = if selected.partial { 206 } else { 200 };
    let reason = if selected.partial {
        "Partial Content"
    } else {
        "OK"
    };
    let mut headers = vec![
        ("Accept-Ranges", "bytes".into()),
        ("Content-Length", selected.length.to_string()),
        ("Content-Type", entry.mime.into()),
    ];
    if selected.partial {
        headers.push((
            "Content-Range",
            format!(
                "bytes {}-{}/{}",
                selected.start,
                selected.start + selected.length - 1,
                entry.length
            ),
        ));
    }
    if write_response_head(&mut stream, status, reason, &headers).is_err()
        || request.method == "HEAD"
        || selected.length == 0
    {
        return;
    }

    match stream_exact_range(
        &mut file,
        &mut stream,
        selected.length,
        entry.revoked.as_ref(),
    ) {
        Ok(StreamOutcome::Complete | StreamOutcome::Revoked) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            owner.revoke_if_current(token, &entry);
        }
        Err(_) => {}
    }
}

struct HttpRequest {
    method: String,
    path: String,
    range: Option<String>,
}

fn read_request(stream: &mut TcpStream, authority: &str) -> Result<HttpRequest, u16> {
    let mut bytes = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut buffer).map_err(|_| 400_u16)?;
        if read == 0 {
            return Err(400);
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_REQUEST_HEADER_BYTES {
            return Err(431);
        }
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
    };
    let text = std::str::from_utf8(&bytes[..header_end]).map_err(|_| 400_u16)?;
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next().ok_or(400_u16)?.split_whitespace();
    let method = request_line.next().ok_or(400_u16)?.to_string();
    let path = request_line.next().ok_or(400_u16)?.to_string();
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        return Err(400);
    }
    let mut host = None;
    let mut range = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(400_u16)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("host") {
            if host.replace(value).is_some() {
                return Err(400);
            }
        } else if name.eq_ignore_ascii_case("range") && range.replace(value.to_string()).is_some() {
            return Err(400);
        }
    }
    if host != Some(authority) {
        return Err(400);
    }
    Ok(HttpRequest {
        method,
        path,
        range,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectedRange {
    length: u64,
    partial: bool,
    start: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeError {
    Malformed,
    Multiple,
    Unsatisfiable,
}

fn select_range(value: Option<&str>, length: u64) -> Result<SelectedRange, RangeError> {
    let Some(value) = value else {
        return Ok(SelectedRange {
            length,
            partial: false,
            start: 0,
        });
    };
    let Some(specification) = value.strip_prefix("bytes=") else {
        return Err(RangeError::Malformed);
    };
    if specification.contains(',') {
        return Err(RangeError::Multiple);
    }
    let (start, end) = specification.split_once('-').ok_or(RangeError::Malformed)?;
    if length == 0 {
        return Err(RangeError::Unsatisfiable);
    }
    if start.is_empty() {
        let suffix = parse_decimal_u64(end)?;
        if suffix == 0 {
            return Err(RangeError::Unsatisfiable);
        }
        let selected_length = suffix.min(length);
        return Ok(SelectedRange {
            length: selected_length,
            partial: true,
            start: length - selected_length,
        });
    }

    let start = parse_decimal_u64(start)?;
    if start >= length {
        return Err(RangeError::Unsatisfiable);
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        let end = parse_decimal_u64(end)?;
        if end < start {
            return Err(RangeError::Malformed);
        }
        end.min(length - 1)
    };
    Ok(SelectedRange {
        length: end - start + 1,
        partial: true,
        start,
    })
}

fn parse_decimal_u64(value: &str) -> Result<u64, RangeError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RangeError::Malformed);
    }
    value.parse().map_err(|_| RangeError::Malformed)
}

fn write_request_error(stream: &mut TcpStream, status: u16, length: u64) -> std::io::Result<()> {
    match status {
        416 => write_empty_response(
            stream,
            status,
            "Range Not Satisfiable",
            &[("Content-Range", format!("bytes */{length}"))],
        ),
        431 => write_empty_response(stream, status, "Request Header Fields Too Large", &[]),
        _ => write_empty_response(stream, status, "Bad Request", &[]),
    }
}

fn write_empty_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    extra_headers: &[(&str, String)],
) -> std::io::Result<()> {
    let mut headers = vec![("Content-Length", "0".into())];
    headers.extend_from_slice(extra_headers);
    write_response_head(stream, status, reason, &headers)
}

fn write_response_head(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(&str, String)],
) -> std::io::Result<()> {
    let mut response = format!("HTTP/1.1 {status} {reason}\r\n");
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("Access-Control-Allow-Origin: *\r\n");
    response.push_str("Cache-Control: no-store\r\n");
    response.push_str("X-Content-Type-Options: nosniff\r\n");
    response.push_str("Connection: close\r\n\r\n");
    stream.write_all(response.as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamOutcome {
    Complete,
    Revoked,
}

pub(crate) fn stream_exact_range<R: Read, W: Write>(
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

enum AdmissionOpenError {
    Internal,
    Unavailable,
}

fn open_admitted_file(entry: &MediaEntry) -> Result<File, AdmissionOpenError> {
    let file = open_no_follow(&entry.path).map_err(|_| AdmissionOpenError::Unavailable)?;
    let snapshot = file_snapshot(&file).map_err(|_| AdmissionOpenError::Internal)?;
    if snapshot.identity != entry.identity || snapshot.length != entry.length {
        return Err(AdmissionOpenError::Unavailable);
    }
    Ok(file)
}

fn file_snapshot(file: &File) -> Result<MediaSourceFingerprint, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err("Choose a supported audio or video file.".into());
    }
    #[cfg(windows)]
    if std::os::windows::fs::MetadataExt::file_attributes(&metadata) & FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err("Recording playback rejects reparse points.".into());
    }
    Ok(MediaSourceFingerprint {
        identity: file_identity(file)?,
        length: metadata.len(),
    })
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file
        .metadata()
        .map_err(|error| format!("Failed to inspect recording file identity: {error}"))?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    file_index: u64,
    volume_serial: u32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ByHandleFileInformation {
    file_attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(
        file: *mut std::ffi::c_void,
        information: *mut ByHandleFileInformation,
    ) -> i32;
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::windows::io::AsRawHandle;

    let mut information = ByHandleFileInformation::default();
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if succeeded == 0 {
        return Err(format!(
            "Failed to inspect recording file identity: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(FileIdentity {
        file_index: (u64::from(information.file_index_high) << 32)
            | u64::from(information.file_index_low),
        volume_serial: information.volume_serial_number,
    })
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity;

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> Result<FileIdentity, String> {
    Err("Secure media file identity is unsupported on this platform.".into())
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(target_os = "linux")]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x0002_0000;
    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

#[cfg(target_os = "macos")]
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    const O_NOFOLLOW: i32 = 0x0000_0100;
    OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn open_no_follow(_path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure no-follow media open is unsupported on this platform",
    ))
}

fn media_mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("mp3") {
        Some("audio/mpeg")
    } else if extension.eq_ignore_ascii_case("m4a") {
        Some("audio/mp4")
    } else if extension.eq_ignore_ascii_case("wav") {
        Some("audio/wav")
    } else if extension.eq_ignore_ascii_case("mp4") {
        Some("video/mp4")
    } else if extension.eq_ignore_ascii_case("flac") {
        Some("audio/flac")
    } else if extension.eq_ignore_ascii_case("ogg") {
        Some("audio/ogg")
    } else if extension.eq_ignore_ascii_case("webm") {
        Some("video/webm")
    } else {
        None
    }
}

fn token_from_url(url: &str, authority: &str) -> Option<String> {
    let path = url.strip_prefix(&format!("http://{authority}"))?;
    token_from_request_path(path).map(str::to_string)
}

fn token_from_request_path(path: &str) -> Option<&str> {
    let token = path.strip_prefix("/media/")?;
    if token.len() == TOKEN_HEX_LENGTH && token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(token)
    } else {
        None
    }
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    fill_secure_random(&mut bytes)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(TOKEN_HEX_LENGTH);
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(token)
}

#[cfg(windows)]
#[link(name = "advapi32")]
unsafe extern "system" {
    #[link_name = "SystemFunction036"]
    fn rtl_gen_random(buffer: *mut std::ffi::c_void, length: u32) -> u8;
}

#[cfg(windows)]
fn fill_secure_random(bytes: &mut [u8]) -> Result<(), String> {
    let length = u32::try_from(bytes.len()).map_err(|_| "Random request is too large.")?;
    let succeeded = unsafe { rtl_gen_random(bytes.as_mut_ptr().cast(), length) };
    if succeeded == 0 {
        Err(format!(
            "Failed to mint media admission token: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn fill_secure_random(bytes: &mut [u8]) -> Result<(), String> {
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .map_err(|error| format!("Failed to mint media admission token: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn fill_secure_random(_bytes: &mut [u8]) -> Result<(), String> {
    Err("Secure media admission tokens are unsupported on this platform.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Cursor, Read, Write},
        net::TcpStream,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::Duration,
    };

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(1);

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "yap-media-owner-{name}-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: &str) -> std::path::PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[derive(Debug)]
    struct RawResponse {
        body: Vec<u8>,
        headers: String,
        status: u16,
    }

    fn request(url: &str, method: &str, range: Option<&str>) -> RawResponse {
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
            // received bytes; the strict header/body assertions below still
            // reject an incomplete response.
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

    #[test]
    fn admission_mints_fresh_opaque_urls_and_reclaims_oldest_at_capacity() {
        let directory = TestDirectory::new("opaque-capacity");
        let path = directory.join("private-meeting.wav");
        std::fs::write(&path, b"0123456789").unwrap();
        let owner = MediaOwner::with_capacity_for_test(2);

        let first = owner.admit(&path, 32 * 1024 * 1024).unwrap();
        let second = owner.admit(&path, 32 * 1024 * 1024).unwrap();
        let third = owner.admit(&path, 32 * 1024 * 1024).unwrap();

        assert_ne!(first.url, second.url);
        assert_ne!(second.url, third.url);
        assert!(!first.url.contains("private-meeting"));
        assert!(!first.url.contains(".wav"));
        assert_eq!(owner.active_admission_count_for_test(), 2);
        assert_eq!(request(&first.url, "GET", None).status, 404);
        assert_eq!(request(&third.url, "GET", None).body, b"0123456789");
    }

    #[test]
    fn release_revokes_a_url_and_is_idempotent() {
        let directory = TestDirectory::new("release");
        let path = directory.join("meeting.wav");
        std::fs::write(&path, b"RIFFpayload").unwrap();
        let owner = MediaOwner::with_capacity_for_test(4);
        let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();

        assert!(owner.release(&admission.url));
        assert!(!owner.release(&admission.url));
        assert_eq!(request(&admission.url, "GET", None).status, 404);
        assert_eq!(owner.active_admission_count_for_test(), 0);
    }

    #[test]
    fn a_replacement_at_the_admitted_path_is_rejected_and_revoked() {
        let directory = TestDirectory::new("replacement");
        let path = directory.join("meeting.wav");
        let original = directory.join("original.wav");
        std::fs::write(&path, b"original bytes").unwrap();
        let owner = MediaOwner::with_capacity_for_test(4);
        let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();
        std::fs::rename(&path, &original).unwrap();
        std::fs::write(&path, b"replacement bytes").unwrap();

        assert_eq!(request(&admission.url, "GET", None).status, 410);
        assert_eq!(request(&admission.url, "GET", None).status, 404);
        assert_eq!(owner.active_admission_count_for_test(), 0);
    }

    #[test]
    fn preprocessing_opens_the_exact_validated_source_without_following_replacements() {
        let directory = TestDirectory::new("preprocessing-source");
        let path = directory.join("meeting.wav");
        let original = directory.join("original.wav");
        std::fs::write(&path, b"original bytes").unwrap();
        let fingerprint = inspect_media_source(&path).unwrap();

        let opened = open_unchanged_media_source(&path, &fingerprint).unwrap();
        assert_eq!(opened.metadata().unwrap().len(), 14);
        drop(opened);

        std::fs::rename(&path, &original).unwrap();
        std::fs::write(&path, b"replacement bytes").unwrap();
        assert!(open_unchanged_media_source(&path, &fingerprint).is_err());
    }

    #[test]
    fn head_and_single_ranges_report_exact_lengths_and_media_headers() {
        let directory = TestDirectory::new("ranges");
        let path = directory.join("meeting.wav");
        std::fs::write(&path, b"0123456789").unwrap();
        let owner = MediaOwner::with_capacity_for_test(4);
        let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();

        let head = request(&admission.url, "HEAD", None);
        assert_eq!(head.status, 200);
        assert!(head.body.is_empty());
        assert!(head.headers.contains("Content-Length: 10"));
        assert!(head.headers.contains("Content-Type: audio/wav"));
        assert!(head.headers.contains("Accept-Ranges: bytes"));

        let middle = request(&admission.url, "GET", Some("bytes=2-5"));
        assert_eq!(middle.status, 206);
        assert_eq!(middle.body, b"2345");
        assert!(middle.headers.contains("Content-Range: bytes 2-5/10"));

        assert_eq!(
            request(&admission.url, "GET", Some("bytes=7-")).body,
            b"789"
        );
        assert_eq!(
            request(&admission.url, "GET", Some("bytes=-3")).body,
            b"789"
        );
    }

    #[test]
    fn malformed_multi_range_and_eof_requests_fail_without_media_bytes() {
        let directory = TestDirectory::new("bad-ranges");
        let path = directory.join("meeting.webm");
        std::fs::write(&path, b"0123456789").unwrap();
        let owner = MediaOwner::with_capacity_for_test(4);
        let admission = owner.admit(&path, 32 * 1024 * 1024).unwrap();

        let malformed = request(&admission.url, "GET", Some("bytes=wat"));
        assert_eq!(malformed.status, 400);
        assert!(malformed.body.is_empty());

        let multiple = request(&admission.url, "GET", Some("bytes=0-1,4-5"));
        assert_eq!(multiple.status, 416);
        assert!(multiple.body.is_empty());

        let eof = request(&admission.url, "GET", Some("bytes=10-"));
        assert_eq!(eof.status, 416);
        assert!(eof.headers.contains("Content-Range: bytes */10"));
        assert!(eof.body.is_empty());
    }

    struct CappedWriter {
        fail_after: usize,
        largest_write: usize,
        written: usize,
    }

    impl Write for CappedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.largest_write = self.largest_write.max(bytes.len());
            if self.written >= self.fail_after {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "client disconnected",
                ));
            }
            self.written += bytes.len();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn large_copy_uses_bounded_chunks_and_stops_on_disconnect_or_revoke() {
        let payload = vec![7_u8; STREAM_BUFFER_BYTES * 4];
        let mut reader = Cursor::new(payload.clone());
        let mut writer = CappedWriter {
            fail_after: STREAM_BUFFER_BYTES,
            largest_write: 0,
            written: 0,
        };
        let revoked = AtomicBool::new(false);

        let error = stream_exact_range(&mut reader, &mut writer, payload.len() as u64, &revoked)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        assert!(writer.largest_write <= STREAM_BUFFER_BYTES);
        assert!(reader.position() <= (STREAM_BUFFER_BYTES * 2) as u64);

        let mut reader = Cursor::new(payload);
        let mut sink = Vec::new();
        revoked.store(true, Ordering::Release);
        let outcome = stream_exact_range(&mut reader, &mut sink, u64::MAX, &revoked).unwrap();
        assert_eq!(outcome, StreamOutcome::Revoked);
        assert_eq!(reader.position(), 0);
        assert!(sink.is_empty());
    }

    #[test]
    fn admission_preserves_u64_length_as_decimal_and_fails_closed_for_waveforms() {
        let length = 9_007_199_254_740_993_u64;
        let (byte_length, waveform_eligible) = admission_metadata(length, 32 * 1024 * 1024);

        assert_eq!(byte_length, "9007199254740993");
        assert!(!waveform_eligible);
    }
}
