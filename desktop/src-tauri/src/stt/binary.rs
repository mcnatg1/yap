use std::io::Write;
use std::path::{Path, PathBuf};

use crate::stt::error::SttError;
use crate::stt::model::{download_file, verify_sha256};
use crate::stt::pin::CrispasrPin;
use crate::stt::sidecar::sidecar_binary_path;

const MIN_BINARY_BYTES: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryInstallStatus {
    Installed,
    Downloadable,
    Invalid,
    Unsupported,
}

impl BinaryInstallStatus {
    pub fn label(self) -> &'static str {
        match self {
            BinaryInstallStatus::Installed => "Installed",
            BinaryInstallStatus::Downloadable => "Optional local fallback not installed",
            BinaryInstallStatus::Invalid => "Invalid local fallback binary",
            BinaryInstallStatus::Unsupported => "Manual install required",
        }
    }
}

pub fn bin_dir_from<F>(env: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(dir) = env("YAP_BIN_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(local) = env("LOCALAPPDATA") {
        return PathBuf::from(local).join("Yap").join("bin");
    }
    PathBuf::from(".").join("bin")
}

pub fn bin_dir() -> PathBuf {
    bin_dir_from(|key| std::env::var(key).ok())
}

pub fn cached_binary_path(version: &str) -> PathBuf {
    let name = if cfg!(windows) {
        format!("crispasr-{version}.exe")
    } else {
        format!("crispasr-{version}")
    };
    bin_dir().join(name)
}

pub fn release_url(version: &str, asset: &str) -> String {
    format!("https://github.com/CrispStrobe/CrispASR/releases/download/v{version}/{asset}")
}

struct PlatformRelease<'a> {
    asset: &'a str,
    member: &'a str,
}

fn platform_release() -> Option<PlatformRelease<'static>> {
    #[cfg(windows)]
    {
        return Some(PlatformRelease {
            asset: "crispasr-windows-x86_64-cpu.zip",
            member: "crispasr-windows-x86_64-cpu/crispasr.exe",
        });
    }
    #[cfg(target_os = "macos")]
    {
        return Some(PlatformRelease {
            asset: "crispasr-macos.tar.gz",
            member: "crispasr-macos/crispasr",
        });
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return Some(PlatformRelease {
            asset: "crispasr-linux-x86_64.tar.gz",
            member: "crispasr-linux-x86_64/crispasr",
        });
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

pub fn is_verified_binary(path: &Path, expected_hash: &str) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if metadata.len() < MIN_BINARY_BYTES {
        return false;
    }
    verify_sha256(path, expected_hash).is_ok()
}

pub fn binary_install_status(exe_dir: &Path) -> Result<BinaryInstallStatus, SttError> {
    let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;

    if dev_override_path().is_some_and(|path| is_verified_binary(&path, &pin.binary_sha256)) {
        return Ok(BinaryInstallStatus::Installed);
    }

    let bundled = sidecar_binary_path(exe_dir);
    if is_verified_binary(&bundled, &pin.binary_sha256) {
        return Ok(BinaryInstallStatus::Installed);
    }

    let cached = cached_binary_path(&pin.crispasr_version);
    if is_verified_binary(&cached, &pin.binary_sha256) {
        return Ok(BinaryInstallStatus::Installed);
    }

    if bundled.exists() && bundled.metadata().map(|meta| meta.len()).unwrap_or(0) < MIN_BINARY_BYTES {
        return Ok(BinaryInstallStatus::Invalid);
    }

    if cached.exists() {
        return Ok(BinaryInstallStatus::Invalid);
    }

    if platform_release().is_some() {
        Ok(BinaryInstallStatus::Downloadable)
    } else {
        Ok(BinaryInstallStatus::Unsupported)
    }
}

fn dev_override_path() -> Option<PathBuf> {
    std::env::var("YAP_CRISPASR_BIN")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.exists())
}

pub fn resolve_for_spawn(exe_dir: &Path) -> Result<PathBuf, SttError> {
    let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;

    if let Some(path) = dev_override_path() {
        if is_verified_binary(&path, &pin.binary_sha256) {
            return Ok(path);
        }
        crate::stt::log_stt("YAP_CRISPASR_BIN failed SHA-256 verification");
        return Err(SttError::SidecarUnreachable);
    }

    let bundled = sidecar_binary_path(exe_dir);
    if is_verified_binary(&bundled, &pin.binary_sha256) {
        return Ok(bundled);
    }

    let cached = cached_binary_path(&pin.crispasr_version);
    if is_verified_binary(&cached, &pin.binary_sha256) {
        return Ok(cached);
    }

    ensure_binary_at(&pin, download_file)
}

pub fn ensure_binary() -> Result<PathBuf, SttError> {
    let pin = crate::stt::pin::load_pin().map_err(|_| SttError::ModelCorrupt)?;
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));

    if let Some(path) = dev_override_path() {
        if is_verified_binary(&path, &pin.binary_sha256) {
            return Ok(path);
        }
        return Err(SttError::SidecarUnreachable);
    }

    let bundled = sidecar_binary_path(&exe_dir);
    if is_verified_binary(&bundled, &pin.binary_sha256) {
        return Ok(bundled);
    }

    ensure_binary_at(&pin, download_file)
}

pub fn ensure_binary_at<D>(pin: &CrispasrPin, mut download: D) -> Result<PathBuf, SttError>
where
    D: FnMut(&str, &Path) -> Result<(), SttError>,
{
    let dest = cached_binary_path(&pin.crispasr_version);
    if is_verified_binary(&dest, &pin.binary_sha256) {
        return Ok(dest);
    }

    if dest.exists() {
        let _ = std::fs::remove_file(&dest);
        let _ = std::fs::remove_file(verified_marker_path(&dest));
    }

    let release = platform_release().ok_or(SttError::SidecarUnreachable)?;
    let url = release_url(&pin.crispasr_version, release.asset);
    crate::stt::log_stt(&format!("crispasr downloading binary from {url}"));

    std::fs::create_dir_all(bin_dir()).map_err(|_| SttError::ModelMissing)?;
    let archive_path = bin_dir().join(release.asset);
    let archive_part = archive_path.with_extension("part");
    download(&url, &archive_part)?;
    std::fs::rename(&archive_part, &archive_path).map_err(|_| SttError::ModelMissing)?;

    let extracted_part = dest.with_extension("part");
    extract_member(&archive_path, release.member, &extracted_part)?;
    std::fs::rename(&extracted_part, &dest).map_err(|_| SttError::ModelMissing)?;
    let _ = std::fs::remove_file(&archive_path);

    match verify_sha256(&dest, &pin.binary_sha256) {
        Ok(()) => {
            write_verified_marker(&dest, &pin.binary_sha256)?;
            crate::stt::log_stt(&format!("crispasr binary installed at {}", dest.display()));
            Ok(dest)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&dest);
            let _ = std::fs::remove_file(verified_marker_path(&dest));
            crate::stt::log_stt("crispasr downloaded binary failed SHA-256 verification; refusing install");
            Err(err)
        }
    }
}

fn verified_marker_path(binary: &Path) -> PathBuf {
    binary.with_extension("verified")
}

fn write_verified_marker(binary: &Path, expected_hash: &str) -> Result<(), SttError> {
    let metadata = std::fs::metadata(binary).map_err(|_| SttError::ModelMissing)?;
    let marker = verified_marker_path(binary);
    std::fs::write(marker, format!("{expected_hash}\n{}\n", metadata.len())).map_err(|_| SttError::ModelMissing)
}

fn extract_member(archive: &Path, member: &str, dest: &Path) -> Result<(), SttError> {
    #[cfg(windows)]
    {
        return extract_member_from_zip(archive, member, dest);
    }
    #[cfg(not(windows))]
    {
        let _ = (archive, member, dest);
        Err(SttError::SidecarUnreachable)
    }
}

#[cfg(windows)]
fn extract_member_from_zip(archive: &Path, member: &str, dest: &Path) -> Result<(), SttError> {
    let file = std::fs::File::open(archive).map_err(|_| SttError::ModelMissing)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| SttError::ModelCorrupt)?;
    let mut entry = zip.by_name(member).map_err(|_| SttError::ModelCorrupt)?;
    let tmp = dest.with_extension("extract.part");
    let mut out = std::fs::File::create(&tmp).map_err(|_| SttError::ModelMissing)?;
    std::io::copy(&mut entry, &mut out).map_err(|_| SttError::ModelMissing)?;
    out.flush().map_err(|_| SttError::ModelMissing)?;
    drop(out);
    std::fs::rename(&tmp, dest).map_err(|_| SttError::ModelMissing)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_url_uses_tag_and_asset() {
        assert_eq!(
            release_url("0.6.12", "crispasr-windows-x86_64-cpu.zip"),
            "https://github.com/CrispStrobe/CrispASR/releases/download/v0.6.12/crispasr-windows-x86_64-cpu.zip"
        );
    }

    #[test]
    fn rejects_stub_sized_binary() {
        let dir = std::env::temp_dir().join(format!("yap-bin-stub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("crispasr.exe");
        std::fs::write(&path, []).unwrap();
        assert!(!is_verified_binary(&path, &"a".repeat(64)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn ensure_binary_at_downloads_extracts_and_verifies() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let root = std::env::temp_dir().join(format!("yap-bin-install-{}", std::process::id()));
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::env::set_var("YAP_BIN_DIR", &bin_dir);

        let payload = vec![b'h'; 1024];
        let hello = crate::stt::model::sha256_file(&{
            let path = root.join("payload.bin");
            std::fs::write(&path, &payload).unwrap();
            path
        })
        .unwrap();

        let pin = CrispasrPin {
            crispasr_version: "0.6.12".into(),
            binary_sha256: hello,
            gguf_repo: "owner/repo".into(),
            gguf_revision: "rev".into(),
            gguf_file: "m.gguf".into(),
            gguf_sha256: "b".repeat(64),
            tokenizer_file: "tokenizer.bin".into(),
            tokenizer_sha256: "c".repeat(64),
        };

        let archive = bin_dir.join("crispasr-windows-x86_64-cpu.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = ZipWriter::new(file);
            zip.start_file(
                "crispasr-windows-x86_64-cpu/crispasr.exe",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&payload).unwrap();
            zip.finish().unwrap();
        }

        let dest = ensure_binary_at(&pin, |url, path| {
            assert!(url.contains("v0.6.12"));
            std::fs::copy(&archive, path).map(|_| ()).map_err(|_| SttError::ModelMissing)
        })
        .unwrap();

        assert!(dest.exists());
        assert!(is_verified_binary(&dest, &pin.binary_sha256));
        std::env::remove_var("YAP_BIN_DIR");
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(windows)]
    #[test]
    fn extract_member_from_zip_writes_executable_bytes() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let dir = std::env::temp_dir().join(format!("yap-bin-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("bundle.zip");
        let dest = dir.join("crispasr.exe");

        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = ZipWriter::new(file);
            zip.start_file(
                "crispasr-windows-x86_64-cpu/crispasr.exe",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"hello").unwrap();
            zip.finish().unwrap();
        }

        extract_member_from_zip(&archive, "crispasr-windows-x86_64-cpu/crispasr.exe", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        std::fs::remove_dir_all(&dir).ok();
    }
}
