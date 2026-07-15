use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant as StdInstant},
};

use tokio::time::{sleep_until, Instant};

use crate::stt::error::SttError;

use super::{
    integrity::verify_download,
    io_error_to_stt,
    operation::DownloadOperation,
    progress::{BodyProgress, DownloadProgress},
    reqwest_error_to_stt,
    temp::{cleanup_stale_download_temps, OperationTemp},
};

const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_REQUEST_HEADERS_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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
        verify_download(temp.path(), request, operation)?;
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
