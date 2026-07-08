use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::stt::error::SttError;

const DOWNLOAD_BUFFER_BYTES: usize = 64 * 1024;

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

pub fn models_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = env("YAP_MODELS_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(local) = env("LOCALAPPDATA") {
        return PathBuf::from(local).join("Yap").join("models");
    }
    PathBuf::from(".").join("models")
}

pub fn models_dir() -> PathBuf {
    models_dir_from(|key| std::env::var(key).ok())
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        hex.push_str(&format!("{byte:02x}"));
    }
    Ok(hex)
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

pub fn download_file(url: &str, dest: &Path) -> Result<(), SttError> {
    download_file_with_progress(url, dest, |_| {}, || false)
}

pub fn download_file_with_progress<P, C>(
    url: &str,
    dest: &Path,
    on_progress: P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    P: FnMut(DownloadProgress),
    C: Fn() -> bool,
{
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|_| SttError::ModelMissing)?;
    let mut response = client.get(url).send().map_err(|_| SttError::ModelMissing)?;
    if !response.status().is_success() {
        return Err(SttError::ModelMissing);
    }
    let total_bytes = response.content_length();
    stream_to_destination(
        &mut response,
        total_bytes,
        dest,
        on_progress,
        is_cancelled,
    )
}

fn progress_metrics(
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    elapsed_ms: u128,
) -> (Option<f32>, Option<f32>) {
    let percent = total_bytes.and_then(|total| {
        (total > 0)
            .then(|| ((downloaded_bytes as f32 / total as f32) * 100.0).clamp(0.0, 100.0))
    });
    let speed_mbps = (elapsed_ms > 0).then(|| {
        let elapsed_seconds = elapsed_ms as f32 / 1000.0;
        ((downloaded_bytes as f32 * 8.0) / elapsed_seconds) / 1_000_000.0
    });
    (percent, speed_mbps)
}

fn stream_to_destination<R, P, C>(
    reader: &mut R,
    total_bytes: Option<u64>,
    dest: &Path,
    mut on_progress: P,
    is_cancelled: C,
) -> Result<(), SttError>
where
    R: Read,
    P: FnMut(DownloadProgress),
    C: Fn() -> bool,
{
    let tmp = dest.with_extension("part");
    let result = (|| {
        let mut file = std::fs::File::create(&tmp).map_err(|_| SttError::ModelMissing)?;
        let mut buffer = [0u8; DOWNLOAD_BUFFER_BYTES];
        let mut downloaded_bytes = 0u64;
        let started_at = Instant::now();

        loop {
            if is_cancelled() {
                return Err(SttError::ModelInstallCancelled);
            }

            let read = reader
                .read(&mut buffer)
                .map_err(|_| SttError::ModelMissing)?;
            if read == 0 {
                break;
            }

            file.write_all(&buffer[..read])
                .map_err(|_| SttError::ModelMissing)?;
            downloaded_bytes += read as u64;
            on_progress(DownloadProgress {
                downloaded_bytes,
                total_bytes,
                elapsed_ms: started_at.elapsed().as_millis(),
            });

            if is_cancelled() {
                return Err(SttError::ModelInstallCancelled);
            }
        }

        drop(file);

        if is_cancelled() {
            return Err(SttError::ModelInstallCancelled);
        }

        std::fs::rename(&tmp, dest).map_err(|_| SttError::ModelMissing)?;
        on_progress(DownloadProgress {
            downloaded_bytes,
            total_bytes,
            elapsed_ms: started_at.elapsed().as_millis(),
        });
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{Error, ErrorKind};

    struct ChunkedReader {
        chunks: Vec<Result<Vec<u8>, std::io::Error>>,
        chunk_index: usize,
        offset: usize,
    }

    impl ChunkedReader {
        fn from_bytes(chunks: &[&[u8]]) -> Self {
            Self {
                chunks: chunks
                    .iter()
                    .map(|chunk| Ok(chunk.to_vec()))
                    .collect::<Vec<_>>(),
                chunk_index: 0,
                offset: 0,
            }
        }

        fn with_error_after(chunks: &[&[u8]], kind: ErrorKind) -> Self {
            let mut all = chunks
                .iter()
                .map(|chunk| Ok(chunk.to_vec()))
                .collect::<Vec<_>>();
            all.push(Err(Error::new(kind, "forced read failure")));
            Self {
                chunks: all,
                chunk_index: 0,
                offset: 0,
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                let Some(entry) = self.chunks.get_mut(self.chunk_index) else {
                    return Ok(0);
                };

                match entry {
                    Ok(chunk) => {
                        if self.offset >= chunk.len() {
                            self.chunk_index += 1;
                            self.offset = 0;
                            continue;
                        }

                        let len = (chunk.len() - self.offset).min(buf.len());
                        buf[..len].copy_from_slice(&chunk[self.offset..self.offset + len]);
                        self.offset += len;
                        if self.offset >= chunk.len() {
                            self.chunk_index += 1;
                            self.offset = 0;
                        }
                        return Ok(len);
                    }
                    Err(_) => {
                        let err = std::mem::replace(
                            entry,
                            Ok(Vec::new()),
                        )
                        .err()
                        .unwrap();
                        self.chunk_index += 1;
                        return Err(err);
                    }
                }
            }
        }
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let unique = format!("{}-{}", std::process::id(), rand_suffix());
            let path = std::env::temp_dir().join(format!("{prefix}-{unique}"));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).ok();
        }
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }

    #[test]
    fn models_dir_prefers_override() {
        let dir = models_dir_from(|key| match key {
            "YAP_MODELS_DIR" => Some("D:/custom".into()),
            _ => None,
        });
        assert_eq!(dir, std::path::PathBuf::from("D:/custom"));
    }

    #[test]
    fn models_dir_falls_back_to_localappdata() {
        let dir = models_dir_from(|key| match key {
            "LOCALAPPDATA" => Some("C:/Users/me/AppData/Local".into()),
            _ => None,
        });
        assert_eq!(
            dir,
            std::path::PathBuf::from("C:/Users/me/AppData/Local")
                .join("Yap")
                .join("models")
        );
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
        let dir = std::env::temp_dir().join(format!("yap-model-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("model.bin");
        std::fs::write(&file, b"hello").unwrap();

        assert_eq!(
            sha256_file(&file).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert!(verify_sha256(
            &file,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        )
        .is_ok());
        assert_eq!(
            verify_sha256(&file, "bad").unwrap_err(),
            SttError::ModelCorrupt
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_sha256_missing_file_is_model_missing() {
        assert_eq!(
            verify_sha256(Path::new("missing.bin"), "abc").unwrap_err(),
            SttError::ModelMissing
        );
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

        let unknown_total = DownloadProgress {
            downloaded_bytes: 128,
            total_bytes: None,
            elapsed_ms: 0,
        };
        assert_eq!(unknown_total.percent(), None);
        assert_eq!(unknown_total.speed_mbps(), None);

        assert_eq!(progress_metrics(300, Some(100), 1).0, Some(100.0));
    }

    #[test]
    fn stream_to_destination_emits_chunk_and_final_progress() {
        let dir = TestDir::new("yap-download-progress");
        let dest = dir.path().join("model.bin");
        let mut reader = ChunkedReader::from_bytes(&[b"abc", b"de"]);
        let mut events = Vec::new();

        stream_to_destination(
            &mut reader,
            Some(5),
            &dest,
            |progress| events.push(progress),
            || false,
        )
        .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"abcde");
        assert!(!dest.with_extension("part").exists());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].downloaded_bytes, 3);
        assert_eq!(events[1].downloaded_bytes, 5);
        assert_eq!(events[2].downloaded_bytes, 5);
        assert_eq!(events[2].percent(), Some(100.0));
    }

    #[test]
    fn stream_to_destination_cleans_partial_file_on_cancel() {
        let dir = TestDir::new("yap-download-cancel");
        let dest = dir.path().join("model.bin");
        let mut reader = ChunkedReader::from_bytes(&[b"abc", b"de"]);
        let seen_progress = Cell::new(false);

        let error = stream_to_destination(
            &mut reader,
            Some(5),
            &dest,
            |_| {
                seen_progress.set(true);
            },
            || seen_progress.get(),
        )
        .unwrap_err();

        assert_eq!(error, SttError::ModelInstallCancelled);
        assert!(!dest.exists());
        assert!(!dest.with_extension("part").exists());
    }

    #[test]
    fn stream_to_destination_cleans_partial_file_on_read_failure() {
        let dir = TestDir::new("yap-download-failure");
        let dest = dir.path().join("model.bin");
        let mut reader = ChunkedReader::with_error_after(&[b"abc"], ErrorKind::ConnectionAborted);

        let error =
            stream_to_destination(&mut reader, Some(5), &dest, |_| {}, || false).unwrap_err();

        assert_eq!(error, SttError::ModelMissing);
        assert!(!dest.exists());
        assert!(!dest.with_extension("part").exists());
    }
}
