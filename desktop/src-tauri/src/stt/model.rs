use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::sync::Notify;
use tokio::time::{sleep_until, Instant};

use crate::stt::error::SttError;

const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_REQUEST_HEADERS_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub elapsed_ms: u128,
}

impl DownloadProgress {
    pub fn percent(self) -> Option<f32> {
        progress_metrics(self.downloaded_bytes, self.total_bytes, self.elapsed_ms).0
    }

    pub fn speed_mbps(self) -> Option<f32> {
        progress_metrics(self.downloaded_bytes, self.total_bytes, self.elapsed_ms).1
    }
}

#[derive(Debug, Clone)]
pub struct DownloadRequest {
    pub url: String,
    pub destination: PathBuf,
    pub expected_bytes: u64,
    pub expected_sha256: String,
}

impl DownloadRequest {
    fn validate(&self) -> Result<(), SttError> {
        let valid_hash = self.expected_sha256.len() == 64
            && self
                .expected_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit());
        if self.expected_bytes == 0 || !valid_hash {
            return Err(SttError::ModelCorrupt);
        }
        if self.destination.file_name().is_none() || self.destination.parent().is_none() {
            return Err(SttError::ModelMissing);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct DownloadOperationInner {
    generation: u64,
    cancelled: AtomicBool,
    cancellation: Notify,
    cleanup_failure: Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct DownloadOperation {
    inner: Arc<DownloadOperationInner>,
}

impl DownloadOperation {
    pub fn new(generation: u64) -> Self {
        Self {
            inner: Arc::new(DownloadOperationInner {
                generation,
                cancelled: AtomicBool::new(false),
                cancellation: Notify::new(),
                cleanup_failure: Mutex::new(None),
            }),
        }
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation
    }

    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            self.inner.cancellation.notify_one();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn take_cleanup_failure(&self) -> Option<String> {
        self.inner
            .cleanup_failure
            .lock()
            .expect("download cleanup state poisoned")
            .take()
    }

    async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            self.inner.cancellation.notified().await;
        }
    }

    fn record_cleanup_failure(&self, message: String) {
        let mut failure = self
            .inner
            .cleanup_failure
            .lock()
            .expect("download cleanup state poisoned");
        match failure.as_mut() {
            Some(existing) => {
                existing.push_str("; ");
                existing.push_str(&message);
            }
            None => *failure = Some(message),
        }
    }
}

pub fn models_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = crate::paths::absolute_env_path(&env, "YAP_MODELS_DIR") {
        return dir;
    }
    crate::paths::app_data_dir_from(env).join("models")
}

pub fn models_dir() -> PathBuf {
    models_dir_from(|key| std::env::var(key).ok())
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(hasher.finalize()))
}

pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), SttError> {
    let actual = sha256_file(path).map_err(|_| SttError::ModelMissing)?;
    actual
        .eq_ignore_ascii_case(expected)
        .then_some(())
        .ok_or(SttError::ModelCorrupt)
}

pub fn hf_resolve_url(repo: &str, revision: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/{revision}/{file}")
}

pub fn download_verified_file<P>(
    request: &DownloadRequest,
    operation: &DownloadOperation,
    mut on_progress: P,
) -> Result<(), SttError>
where
    P: FnMut(DownloadProgress),
{
    request.validate()?;
    let parent = request.destination.parent().ok_or(SttError::ModelMissing)?;
    std::fs::create_dir_all(parent).map_err(io_error_to_stt)?;
    cleanup_stale_download_temps(&request.destination, operation)?;

    let mut temp = OperationTemp::create(&request.destination, operation.clone())?;
    let client = download_client()?;
    let started_at = StdInstant::now();
    let download_result = tauri::async_runtime::block_on(download_to_temp(
        &client,
        request,
        operation,
        temp.file_mut()?,
        &mut on_progress,
        started_at,
    ));

    (|| {
        download_result?;
        temp.sync()?;
        if operation.is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
        verify_download(&temp.path, request, operation)?;
        if operation.is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
        temp.publish_to(&request.destination)
    })()
}

fn download_client() -> Result<reqwest::Client, SttError> {
    reqwest::Client::builder()
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .build()
        .map_err(reqwest_error_to_stt)
}

async fn download_to_temp<P>(
    client: &reqwest::Client,
    request: &DownloadRequest,
    operation: &DownloadOperation,
    file: &mut std::fs::File,
    on_progress: &mut P,
    started_at: StdInstant,
) -> Result<(), SttError>
where
    P: FnMut(DownloadProgress),
{
    let total_deadline = Instant::now() + DOWNLOAD_TOTAL_TIMEOUT;
    tokio::select! {
        biased;
        _ = operation.cancelled() => Err(SttError::ModelInstallCancelled),
        _ = sleep_until(total_deadline) => Err(SttError::Timeout),
        result = download_response_to_temp(client, request, file, on_progress, started_at) => result,
    }
}

async fn download_response_to_temp<P>(
    client: &reqwest::Client,
    request: &DownloadRequest,
    file: &mut std::fs::File,
    on_progress: &mut P,
    started_at: StdInstant,
) -> Result<(), SttError>
where
    P: FnMut(DownloadProgress),
{
    let response_deadline = Instant::now() + DOWNLOAD_REQUEST_HEADERS_TIMEOUT;
    let response = tokio::select! {
        biased;
        _ = sleep_until(response_deadline) => return Err(SttError::Timeout),
        response = client.get(&request.url).send() => response.map_err(reqwest_error_to_stt)?,
    };

    if !response.status().is_success() {
        return Err(SttError::ModelMissing);
    }
    if response
        .content_length()
        .is_some_and(|length| length != request.expected_bytes)
    {
        return Err(SttError::ModelCorrupt);
    }

    stream_body(
        response,
        file,
        request.expected_bytes,
        on_progress,
        started_at,
    )
    .await
}

async fn stream_body<P>(
    mut response: reqwest::Response,
    file: &mut std::fs::File,
    expected_bytes: u64,
    on_progress: &mut P,
    started_at: StdInstant,
) -> Result<(), SttError>
where
    P: FnMut(DownloadProgress),
{
    let now = Instant::now();
    let mut progress = BodyProgress::new(expected_bytes, now, DOWNLOAD_NO_PROGRESS_TIMEOUT);
    loop {
        let chunk = tokio::select! {
            biased;
            _ = sleep_until(progress.deadline()) => return Err(SttError::Timeout),
            chunk = response.chunk() => chunk,
        };

        match chunk {
            Ok(Some(bytes)) => {
                if !progress.record_chunk(&bytes, Instant::now())? {
                    continue;
                }
                file.write_all(&bytes).map_err(io_error_to_stt)?;
                on_progress(DownloadProgress {
                    downloaded_bytes: progress.downloaded_bytes,
                    total_bytes: Some(expected_bytes),
                    elapsed_ms: started_at.elapsed().as_millis(),
                });
            }
            Ok(None) => return progress.finish(),
            Err(_) if progress.downloaded_bytes < expected_bytes => {
                return Err(SttError::ModelCorrupt)
            }
            Err(error) => return Err(reqwest_error_to_stt(error)),
        }
    }
}

#[derive(Debug)]
struct BodyProgress {
    expected_bytes: u64,
    downloaded_bytes: u64,
    timeout: Duration,
    deadline: Instant,
}

impl BodyProgress {
    fn new(expected_bytes: u64, now: Instant, timeout: Duration) -> Self {
        Self {
            expected_bytes,
            downloaded_bytes: 0,
            timeout,
            deadline: now + timeout,
        }
    }

    fn deadline(&self) -> Instant {
        self.deadline
    }

    fn record_chunk(&mut self, bytes: &[u8], now: Instant) -> Result<bool, SttError> {
        if bytes.is_empty() {
            return Ok(false);
        }
        let next = self
            .downloaded_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(SttError::ModelCorrupt)?;
        if next > self.expected_bytes {
            return Err(SttError::ModelCorrupt);
        }
        self.downloaded_bytes = next;
        self.deadline = now + self.timeout;
        Ok(true)
    }

    fn finish(&self) -> Result<(), SttError> {
        (self.downloaded_bytes == self.expected_bytes)
            .then_some(())
            .ok_or(SttError::ModelCorrupt)
    }
}

fn verify_download(
    path: &Path,
    request: &DownloadRequest,
    operation: &DownloadOperation,
) -> Result<(), SttError> {
    let metadata = std::fs::metadata(path).map_err(io_error_to_stt)?;
    if metadata.len() != request.expected_bytes {
        return Err(SttError::ModelCorrupt);
    }
    let mut file = std::fs::File::open(path).map_err(io_error_to_stt)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        if operation.is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }
        let read = file.read(&mut buffer).map_err(io_error_to_stt)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hex_digest(hasher.finalize());
    actual
        .eq_ignore_ascii_case(&request.expected_sha256)
        .then_some(())
        .ok_or(SttError::ModelCorrupt)
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    hex
}

struct OperationTemp {
    path: PathBuf,
    file: Option<std::fs::File>,
    operation: DownloadOperation,
    published: bool,
}

impl OperationTemp {
    fn create(destination: &Path, operation: DownloadOperation) -> Result<Self, SttError> {
        let (path, file) = reserve_operation_temp_file(destination, operation.generation())?;
        Ok(Self {
            path,
            file: Some(file),
            operation,
            published: false,
        })
    }

    fn file_mut(&mut self) -> Result<&mut std::fs::File, SttError> {
        self.file.as_mut().ok_or(SttError::ModelMissing)
    }

    fn sync(&mut self) -> Result<(), SttError> {
        self.file_mut()?.sync_all().map_err(io_error_to_stt)
    }

    fn publish_to(&mut self, destination: &Path) -> Result<(), SttError> {
        self.file.take();
        atomic_replace_same_directory(&self.path, destination)?;
        self.published = true;
        sync_parent_directory(destination).map_err(io_error_to_stt)
    }

    fn cleanup(&mut self) -> Result<(), String> {
        self.file.take();
        if self.published {
            return Ok(());
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{}: {error}", self.path.display())),
        }
    }
}

impl Drop for OperationTemp {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            self.operation.record_cleanup_failure(error);
        }
    }
}

pub(crate) fn cleanup_stale_download_temps(
    destination: &Path,
    operation: &DownloadOperation,
) -> Result<(), SttError> {
    try_cleanup_stale_download_temps(destination).map_err(|message| {
        operation.record_cleanup_failure(message);
        SttError::ModelMissing
    })
}

fn try_cleanup_stale_download_temps(destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", destination.display()))?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no file name", destination.display()))?;
    let legacy_temp = destination.with_extension("part");
    let entries = std::fs::read_dir(parent)
        .map_err(|error| format!("could not inspect {}: {error}", parent.display()))?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "could not inspect an entry in {}: {error}",
                parent.display()
            )
        })?;
        let candidate = entry.path();
        let candidate_name = entry.file_name();
        let candidate_name = candidate_name.to_string_lossy();
        let is_legacy_temp = candidate == legacy_temp && candidate != destination;
        if !is_legacy_temp && !is_download_temp_name(file_name, &candidate_name) {
            continue;
        }

        let metadata = std::fs::symlink_metadata(&candidate)
            .map_err(|error| format!("could not inspect {}: {error}", candidate.display()))?;
        if metadata.is_dir() {
            return Err(format!(
                "refusing to remove directory {}",
                candidate.display()
            ));
        }
        std::fs::remove_file(&candidate)
            .map_err(|error| format!("could not remove {}: {error}", candidate.display()))?;
    }
    Ok(())
}

fn is_download_temp_name(file_name: &str, candidate_name: &str) -> bool {
    let Some(candidate_name) = candidate_name.strip_suffix(".part") else {
        return false;
    };
    let operation_prefix = format!("{file_name}.op-");
    if let Some(components) = candidate_name.strip_prefix(&operation_prefix) {
        return has_numeric_components(components, '-', 4);
    }
    let legacy_prefix = format!("{file_name}.");
    candidate_name
        .strip_prefix(&legacy_prefix)
        .is_some_and(|components| has_numeric_components(components, '.', 3))
}

fn has_numeric_components(value: &str, separator: char, expected: usize) -> bool {
    let mut components = value.split(separator);
    (0..expected).all(|_| {
        components
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) && components.next().is_none()
}

fn reserve_operation_temp_file(
    destination: &Path,
    generation: u64,
) -> Result<(PathBuf, std::fs::File), SttError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(SttError::ModelMissing)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SttError::ModelMissing)?
        .as_nanos();
    let pid = std::process::id();
    for attempt in 0..32 {
        let path = destination.with_file_name(format!(
            "{file_name}.op-{generation}-{pid}-{nonce}-{attempt}.part"
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error_to_stt(error)),
        }
    }
    Err(SttError::ModelMissing)
}

pub(crate) fn write_text_atomically(path: &Path, text: &str) -> Result<(), SttError> {
    let (temp, mut file) = reserve_sibling_temp_file(path)?;
    let result = (|| {
        file.write_all(text.as_bytes()).map_err(io_error_to_stt)?;
        file.sync_all().map_err(io_error_to_stt)?;
        drop(file);
        atomic_replace_same_directory(&temp, path)?;
        sync_parent_directory(path).map_err(io_error_to_stt)
    })();
    if result.is_err() {
        match std::fs::remove_file(&temp) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(io_error_to_stt(error)),
        }
    }
    result
}

fn reserve_sibling_temp_file(path: &Path) -> Result<(PathBuf, std::fs::File), SttError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(SttError::ModelMissing)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SttError::ModelMissing)?
        .as_nanos();
    let pid = std::process::id();
    for attempt in 0..32 {
        let temp = path.with_file_name(format!("{file_name}.{pid}.{nonce}.{attempt}.part"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error_to_stt(error)),
        }
    }
    Err(SttError::ModelMissing)
}

#[cfg(windows)]
fn atomic_replace_same_directory(source: &Path, destination: &Path) -> Result<(), SttError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let source = PCWSTR(source_wide.as_ptr());
    let destination = PCWSTR(destination_wide.as_ptr());

    let result = unsafe {
        if destination_path_exists(destination_wide.as_slice()) {
            ReplaceFileW(
                destination,
                source,
                PCWSTR::null(),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
        } else {
            MoveFileExW(
                source,
                destination,
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    result.map_err(|_| SttError::ModelMissing)
}

#[cfg(windows)]
fn destination_path_exists(wide_path: &[u16]) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    unsafe { GetFileAttributesW(PCWSTR(wide_path.as_ptr())) != INVALID_FILE_ATTRIBUTES }
}

#[cfg(not(windows))]
fn atomic_replace_same_directory(source: &Path, destination: &Path) -> Result<(), SttError> {
    std::fs::rename(source, destination).map_err(io_error_to_stt)
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().ok_or_else(|| {
        std::io::Error::new(ErrorKind::InvalidInput, "path has no parent directory")
    })?)?
    .sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn reqwest_error_to_stt(error: reqwest::Error) -> SttError {
    if error.is_timeout() {
        SttError::Timeout
    } else {
        SttError::ModelMissing
    }
}

fn io_error_to_stt(error: std::io::Error) -> SttError {
    if error.kind() == ErrorKind::TimedOut {
        SttError::Timeout
    } else {
        SttError::ModelMissing
    }
}

fn progress_metrics(
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    elapsed_ms: u128,
) -> (Option<f32>, Option<f32>) {
    let percent = total_bytes.and_then(|total| {
        (total > 0).then(|| ((downloaded_bytes as f32 / total as f32) * 100.0).clamp(0.0, 100.0))
    });
    let speed_mbps = (elapsed_ms > 0).then(|| {
        let elapsed_seconds = elapsed_ms as f32 / 1000.0;
        ((downloaded_bytes as f32 * 8.0) / elapsed_seconds) / 1_000_000.0
    });
    (percent, speed_mbps)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn models_dir_prefers_override() {
        let custom = std::env::temp_dir().join("custom-yap-models");
        let dir = models_dir_from(|key| match key {
            "YAP_MODELS_DIR" => Some(custom.display().to_string()),
            _ => None,
        });
        assert_eq!(dir, custom);
    }

    #[test]
    fn models_dir_falls_back_to_localappdata() {
        let local = std::env::temp_dir().join("local-data");
        let dir = models_dir_from(|key| match key {
            "LOCALAPPDATA" => Some(local.display().to_string()),
            _ => None,
        });
        assert_eq!(dir, local.join("Yap").join("models"));
    }

    #[test]
    fn hf_resolve_url_is_pinned_by_revision() {
        assert_eq!(
            hf_resolve_url("owner/repo", "abc123", "model.onnx"),
            "https://huggingface.co/owner/repo/resolve/abc123/model.onnx"
        );
    }

    #[test]
    fn verify_sha256_matches_and_mismatches() {
        let dir = TestDir::new("yap-sha");
        let file = dir.0.join("model.bin");
        std::fs::write(&file, b"hello").unwrap();
        assert_eq!(
            sha256_file(&file).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(verify_sha256(&file, "bad"), Err(SttError::ModelCorrupt));
    }

    #[test]
    fn progress_metrics_handle_partial_complete_and_zero_elapsed() {
        let half = DownloadProgress {
            downloaded_bytes: 50,
            total_bytes: Some(200),
            elapsed_ms: 250,
        };
        assert_eq!(half.percent(), Some(25.0));
        assert_eq!(half.speed_mbps(), Some(0.0016));
        assert_eq!(progress_metrics(300, Some(100), 1).0, Some(100.0));
        assert_eq!(progress_metrics(1, None, 0), (None, None));
    }

    #[test]
    fn empty_body_chunks_do_not_extend_the_no_progress_deadline() {
        let base = Instant::now();
        let timeout = Duration::from_secs(30);
        let mut progress = BodyProgress::new(6, base, timeout);
        let original_deadline = progress.deadline();

        assert!(!progress
            .record_chunk(&[], base + Duration::from_secs(10))
            .unwrap());
        assert_eq!(progress.deadline(), original_deadline);

        assert!(progress
            .record_chunk(b"abc", base + Duration::from_secs(11))
            .unwrap());
        assert_eq!(
            progress.deadline(),
            base + Duration::from_secs(11) + timeout
        );
    }

    #[test]
    fn atomic_text_write_replaces_existing_marker_without_delete_window() {
        let dir = TestDir::new("yap-marker");
        let marker = dir.0.join("model.verified");
        std::fs::write(&marker, "old").unwrap();

        write_text_atomically(&marker, "new").unwrap();

        assert_eq!(std::fs::read_to_string(marker).unwrap(), "new");
    }

    #[test]
    fn stale_temp_cleanup_failure_is_recorded_without_touching_the_destination() {
        let dir = TestDir::new("yap-stale-cleanup-failure");
        let destination = dir.0.join("model.bin");
        let stale_temp = dir.0.join("model.bin.op-1-2-3-4.part");
        std::fs::write(&destination, b"verified-old").unwrap();
        std::fs::create_dir(&stale_temp).unwrap();
        let operation = DownloadOperation::new(9);

        assert_eq!(
            cleanup_stale_download_temps(&destination, &operation),
            Err(SttError::ModelMissing)
        );
        assert_eq!(std::fs::read(destination).unwrap(), b"verified-old");
        assert!(stale_temp.is_dir());
        assert!(operation
            .take_cleanup_failure()
            .is_some_and(|message| message.contains("refusing to remove directory")));
    }
}
