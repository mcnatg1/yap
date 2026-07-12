use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use reqwest::Url;

pub const CURRENT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerSettings {
    pub schema_version: u16,
    pub enabled: bool,
    pub base_url: Option<String>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            enabled: false,
            base_url: None,
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Invalid(&'static str),
    IncompatibleSchema(u64),
    AccessIo(std::io::Error),
    SaveIo(std::io::Error),
    PublishedButDurabilityUnconfirmed(std::io::Error),
    Serialization(serde_json::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::IncompatibleSchema(version) => write!(
                formatter,
                "Server settings use unsupported schema version {version}."
            ),
            Self::AccessIo(error) => {
                write!(formatter, "Could not access server settings: {error}")
            }
            Self::SaveIo(error) => write!(formatter, "Could not save server settings: {error}"),
            Self::PublishedButDurabilityUnconfirmed(error) => write!(
                formatter,
                "Server settings changed, but durability confirmation failed: {error}"
            ),
            Self::Serialization(error) => {
                write!(formatter, "Could not encode server settings: {error}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl ConfigError {
    pub(crate) fn settings_were_published(&self) -> bool {
        matches!(self, Self::PublishedButDurabilityUnconfirmed(_))
    }
}

impl From<serde_json::Error> for ConfigError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

pub fn validate_base_url(raw: &str, allow_insecure_private: bool) -> Result<String, ConfigError> {
    let url =
        Url::parse(raw.trim()).map_err(|_| ConfigError::Invalid("Enter a valid server URL."))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ConfigError::Invalid(
            "Server URL cannot include credentials.",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ConfigError::Invalid(
            "Server URL cannot include a query or fragment.",
        ));
    }
    if !matches!(url.path(), "" | "/" | "/v1" | "/v1/") {
        return Err(ConfigError::Invalid(
            "Server URL path must be /v1 or empty.",
        ));
    }
    let host = url
        .host_str()
        .ok_or(ConfigError::Invalid("Server URL must include a host."))?;

    match url.scheme() {
        "https" => {}
        "http" if is_loopback_host(host) => {}
        "http" if allow_insecure_private && is_rfc1918_host(host) => {}
        "http" => {
            return Err(ConfigError::Invalid(
                "Use HTTPS unless the server is loopback or approved private development.",
            ));
        }
        _ => return Err(ConfigError::Invalid("Server URL must use HTTPS or HTTP.")),
    }

    Ok(url.origin().ascii_serialization())
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || parse_ip_host(host).is_some_and(|address| address.is_loopback())
}

fn is_rfc1918_host(host: &str) -> bool {
    matches!(parse_ip_host(host), Some(std::net::IpAddr::V4(address)) if address.is_private())
}

fn parse_ip_host(host: &str) -> Option<std::net::IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

pub fn load() -> Result<ServerSettings, ConfigError> {
    load_from_path(&settings_path(), allow_insecure_private_server())
}

pub fn save(settings: &ServerSettings) -> Result<ServerSettings, ConfigError> {
    save_to_path(settings, &settings_path(), allow_insecure_private_server())
}

pub(crate) fn load_from_path(
    path: &Path,
    allow_insecure_private: bool,
) -> Result<ServerSettings, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ServerSettings::default());
        }
        Err(error) => return Err(ConfigError::AccessIo(error)),
    };
    decode_persisted_settings(&text, allow_insecure_private)
}

pub(crate) fn save_to_path(
    settings: &ServerSettings,
    path: &Path,
    allow_insecure_private: bool,
) -> Result<ServerSettings, ConfigError> {
    save_to_path_with_hooks(
        settings,
        path,
        allow_insecure_private,
        || Ok(()),
        || Ok(()),
        |_, _| Ok(()),
        |_| Ok(()),
    )
}

pub(super) fn save_to_path_with_hooks<BeforeLock, AfterLock, BeforePublish, AfterPublish>(
    settings: &ServerSettings,
    path: &Path,
    allow_insecure_private: bool,
    before_lock: BeforeLock,
    after_lock: AfterLock,
    before_publish: BeforePublish,
    after_publish: AfterPublish,
) -> Result<ServerSettings, ConfigError>
where
    BeforeLock: FnOnce() -> std::io::Result<()>,
    AfterLock: FnOnce() -> std::io::Result<()>,
    BeforePublish: FnOnce(&Path, &Path) -> std::io::Result<()>,
    AfterPublish: FnOnce(&Path) -> std::io::Result<()>,
{
    let normalized = normalize_settings(settings, allow_insecure_private)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(ConfigError::SaveIo)?;
    }
    before_lock().map_err(ConfigError::SaveIo)?;
    let _lock = acquire_settings_lock(path)?;
    after_lock().map_err(ConfigError::SaveIo)?;
    ensure_existing_schema_compatible(path)?;
    let encoded = serde_json::to_string_pretty(&normalized)?;
    write_atomically_locked_with_hooks(path, encoded.as_bytes(), before_publish, after_publish)?;
    Ok(normalized)
}

fn decode_persisted_settings(
    text: &str,
    allow_insecure_private: bool,
) -> Result<ServerSettings, ConfigError> {
    let value = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => value,
        Err(_) => return Ok(ServerSettings::default()),
    };
    ensure_schema_compatible(&value)?;
    let settings = match serde_json::from_value::<ServerSettings>(value) {
        Ok(settings) => settings,
        Err(_) => return Ok(ServerSettings::default()),
    };
    normalize_settings(&settings, allow_insecure_private)
}

fn ensure_existing_schema_compatible(path: &Path) -> Result<(), ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ConfigError::AccessIo(error)),
    };
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        ensure_schema_compatible(&value)?;
    }
    Ok(())
}

fn ensure_schema_compatible(value: &serde_json::Value) -> Result<(), ConfigError> {
    if let Some(version) = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
    {
        if version != u64::from(CURRENT_SCHEMA_VERSION) {
            return Err(ConfigError::IncompatibleSchema(version));
        }
    }
    Ok(())
}

fn normalize_settings(
    settings: &ServerSettings,
    allow_insecure_private: bool,
) -> Result<ServerSettings, ConfigError> {
    if settings.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(ConfigError::IncompatibleSchema(u64::from(
            settings.schema_version,
        )));
    }
    let base_url = settings
        .base_url
        .as_deref()
        .map(|raw| validate_base_url(raw, allow_insecure_private))
        .transpose()?;
    if settings.enabled && base_url.is_none() {
        return Err(ConfigError::Invalid("Enter a server URL before enabling."));
    }
    Ok(ServerSettings {
        schema_version: CURRENT_SCHEMA_VERSION,
        enabled: settings.enabled,
        base_url,
    })
}

fn settings_path() -> PathBuf {
    crate::paths::app_data_dir().join("server-settings.json")
}

fn allow_insecure_private_server() -> bool {
    std::env::var("YAP_ALLOW_INSECURE_PRIVATE_SERVER").as_deref() == Ok("1")
}

#[cfg(test)]
fn write_atomically_with_before_publish<F>(
    path: &Path,
    contents: &[u8],
    before_publish: F,
) -> Result<(), ConfigError>
where
    F: FnOnce(&Path, &Path) -> std::io::Result<()>,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(ConfigError::SaveIo)?;
    }
    let _lock = acquire_settings_lock(path)?;
    write_atomically_locked_with_hooks(path, contents, before_publish, |_| Ok(()))
}

fn write_atomically_locked_with_hooks<BeforePublish, AfterPublish>(
    path: &Path,
    contents: &[u8],
    before_publish: BeforePublish,
    after_publish: AfterPublish,
) -> Result<(), ConfigError>
where
    BeforePublish: FnOnce(&Path, &Path) -> std::io::Result<()>,
    AfterPublish: FnOnce(&Path) -> std::io::Result<()>,
{
    let legacy_partial = path.with_extension("json.part");
    match std::fs::remove_file(&legacy_partial) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(ConfigError::SaveIo(error)),
    }
    scavenge_abandoned_unique_partials(path).map_err(ConfigError::SaveIo)?;

    let (partial, mut file) = reserve_unique_partial(path).map_err(ConfigError::SaveIo)?;

    let publication = (|| -> std::io::Result<()> {
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        before_publish(&partial, path)?;
        atomic_replace_same_directory(&partial, path)?;
        Ok(())
    })();

    if let Err(error) = publication {
        std::fs::remove_file(&partial).ok();
        return Err(ConfigError::SaveIo(error));
    }
    if let Err(error) = after_publish(path).and_then(|_| sync_parent_directory(path)) {
        return Err(ConfigError::PublishedButDurabilityUnconfirmed(error));
    }
    Ok(())
}

fn reserve_unique_partial(path: &Path) -> std::io::Result<(PathBuf, std::fs::File)> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no file name",
        )
    })?;
    for _ in 0..64 {
        let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let mut partial_name = file_name.to_os_string();
        partial_name.push(format!(".{}.{counter}.part", std::process::id()));
        let partial = path.with_file_name(partial_name);
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial)
        {
            Ok(file) => return Ok((partial, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve unique server settings partial",
    ))
}

fn scavenge_abandoned_unique_partials(path: &Path) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no parent",
        )
    })?;
    let base_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "settings path has no UTF-8 file name",
            )
        })?;
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let Some(candidate) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_unique_partial_name(base_name, &candidate) || !entry.file_type()?.is_file() {
            continue;
        }
        std::fs::remove_file(entry.path())?;
    }
    Ok(())
}

fn is_unique_partial_name(base_name: &str, candidate: &str) -> bool {
    let Some(identity) = candidate
        .strip_prefix(base_name)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".part"))
    else {
        return false;
    };
    let mut parts = identity.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(pid), Some(counter), None)
            if pid.parse::<u32>().is_ok() && counter.parse::<u64>().is_ok()
    )
}

struct SettingsFileLock {
    file: std::fs::File,
}

fn acquire_settings_lock(path: &Path) -> Result<SettingsFileLock, ConfigError> {
    let lock_path = path.with_extension("json.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(ConfigError::SaveIo)?;
    lock_file_exclusive(&file).map_err(ConfigError::SaveIo)?;
    Ok(SettingsFileLock { file })
}

impl Drop for SettingsFileLock {
    fn drop(&mut self) {
        unlock_file(&self.file).ok();
    }
}

#[cfg(windows)]
fn lock_file_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK};
    use windows::Win32::System::IO::OVERLAPPED;

    let mut overlapped = OVERLAPPED::default();
    unsafe {
        LockFileEx(
            HANDLE(file.as_raw_handle()),
            LOCKFILE_EXCLUSIVE_LOCK,
            None,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    }
    .map_err(|_| std::io::Error::last_os_error())
}

#[cfg(windows)]
fn unlock_file(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::UnlockFileEx;
    use windows::Win32::System::IO::OVERLAPPED;

    let mut overlapped = OVERLAPPED::default();
    unsafe {
        UnlockFileEx(
            HANDLE(file.as_raw_handle()),
            None,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    }
    .map_err(|_| std::io::Error::last_os_error())
}

#[cfg(unix)]
fn lock_file_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_file(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn atomic_replace_same_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileAttributesW, MoveFileExW, ReplaceFileW, INVALID_FILE_ATTRIBUTES,
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, REPLACEFILE_WRITE_THROUGH,
    };

    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let source_wide = wide(source);
    let destination_wide = wide(destination);
    let source = PCWSTR(source_wide.as_ptr());
    let destination = PCWSTR(destination_wide.as_ptr());
    let destination_exists = unsafe { GetFileAttributesW(destination) } != INVALID_FILE_ATTRIBUTES;
    let result = unsafe {
        if destination_exists {
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
    result.map_err(|_| std::io::Error::last_os_error())
}

#[cfg(not(windows))]
fn atomic_replace_same_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent")
    })?)?
    .sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Barrier, Mutex};
    use std::time::{Duration, Instant};

    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "yap-server-settings-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn partial_files(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with("server-settings.json") && name.ends_with(".part"))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    const CROSS_PROCESS_CHILD_PATH: &str = "YAP_TEST_SETTINGS_CHILD_PATH";
    const CROSS_PROCESS_READY_PATH: &str = "YAP_TEST_SETTINGS_READY_PATH";
    const CROSS_PROCESS_RELEASE_PATH: &str = "YAP_TEST_SETTINGS_RELEASE_PATH";
    const CROSS_PROCESS_FUTURE: &str = r#"{
  "schemaVersion": 2,
  "enabled": true,
  "baseUrl": "https://future-process.example",
  "futureField": "preserve-cross-process"
}"#;

    fn wait_for_path(path: &Path, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    fn wait_for_child(mut child: std::process::Child, timeout: Duration) -> std::process::Output {
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait().unwrap() {
                Some(_) => return child.wait_with_output().unwrap(),
                None if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                None => {
                    child.kill().ok();
                    let output = child.wait_with_output().unwrap();
                    panic!(
                        "settings child exceeded {:?}: stdout={} stderr={}",
                        timeout,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
    }

    #[test]
    fn https_urls_normalize_to_an_origin_without_a_v1_suffix() {
        assert_eq!(
            validate_base_url("https://server.example:8443/v1/", false).unwrap(),
            "https://server.example:8443"
        );
        assert_eq!(
            validate_base_url("HTTPS://SERVER.EXAMPLE/", false).unwrap(),
            "https://server.example"
        );
        assert!(validate_base_url("https://server.example/api", false).is_err());
    }

    #[test]
    fn loopback_http_accepts_ipv4_ipv6_and_localhost() {
        assert_eq!(
            validate_base_url("http://127.0.0.1:18765/v1", false).unwrap(),
            "http://127.0.0.1:18765"
        );
        assert_eq!(
            validate_base_url("http://[::1]:18765", false).unwrap(),
            "http://[::1]:18765"
        );
        assert_eq!(
            validate_base_url("http://localhost:18765/", false).unwrap(),
            "http://localhost:18765"
        );
    }

    #[test]
    fn private_http_requires_the_explicit_process_override() {
        for raw in [
            "http://10.4.5.6:18765",
            "http://172.16.4.5:18765",
            "http://192.168.50.1:18765/v1",
        ] {
            assert!(validate_base_url(raw, false).is_err(), "accepted {raw}");
            assert!(validate_base_url(raw, true).is_ok(), "rejected {raw}");
        }
    }

    #[test]
    fn public_http_is_rejected_even_when_private_http_is_allowed() {
        for raw in [
            "http://server.example:18765",
            "http://8.8.8.8:18765",
            "http://169.254.1.2:18765",
        ] {
            assert!(validate_base_url(raw, false).is_err(), "accepted {raw}");
            assert!(validate_base_url(raw, true).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn credentials_queries_and_fragments_are_rejected() {
        for raw in [
            "https://user:secret@server.example",
            "https://server.example?token=secret",
            "https://server.example/#section",
        ] {
            assert!(validate_base_url(raw, false).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn malformed_json_recovers_disabled_defaults() {
        let dir = temp_dir("malformed");
        let path = dir.join("server-settings.json");
        std::fs::write(&path, "{not-json").unwrap();

        assert_eq!(
            load_from_path(&path, false).unwrap(),
            ServerSettings::default()
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_replaces_a_stale_partial_and_persists_only_the_public_schema() {
        let dir = temp_dir("atomic");
        let path = dir.join("server-settings.json");
        let partial = dir.join("server-settings.json.part");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"enabled":false,"baseUrl":null}"#,
        )
        .unwrap();
        std::fs::write(&partial, "stale-secret").unwrap();
        let settings = ServerSettings {
            schema_version: CURRENT_SCHEMA_VERSION,
            enabled: true,
            base_url: Some("https://server.example/v1".into()),
        };

        let saved = save_to_path(&settings, &path, false).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(saved.base_url.as_deref(), Some("https://server.example"));
        assert!(!partial.exists());
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["baseUrl", "enabled", "schemaVersion"]
        );
        assert_eq!(load_from_path(&path, false).unwrap(), saved);
        assert!(!std::fs::read_to_string(path).unwrap().contains("secret"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn concurrent_writers_publish_only_complete_payloads_without_temp_leaks() {
        let dir = temp_dir("concurrent");
        let path = dir.join("server-settings.json");
        let initial = ServerSettings::default();
        save_to_path(&initial, &path, false).unwrap();

        let left = ServerSettings {
            schema_version: CURRENT_SCHEMA_VERSION,
            enabled: true,
            base_url: Some("https://left.example".into()),
        };
        let right = ServerSettings {
            schema_version: CURRENT_SCHEMA_VERSION,
            enabled: true,
            base_url: Some("https://right.example:8443/v1".into()),
        };
        let expected_right = ServerSettings {
            base_url: Some("https://right.example:8443".into()),
            ..right.clone()
        };
        let release = Arc::new(Barrier::new(3));
        let observing = Arc::new(AtomicBool::new(true));
        let observations = Arc::new(Mutex::new(Vec::new()));

        let observer_path = path.clone();
        let observer_running = Arc::clone(&observing);
        let observer_values = Arc::clone(&observations);
        let observer = std::thread::spawn(move || {
            while observer_running.load(Ordering::Acquire) {
                if let Ok(text) = std::fs::read_to_string(&observer_path) {
                    observer_values.lock().unwrap().push(text);
                }
                std::thread::yield_now();
            }
        });

        let spawn_writer = |settings: ServerSettings| {
            let writer_path = path.clone();
            let writer_release = Arc::clone(&release);
            std::thread::spawn(move || {
                save_to_path_with_hooks(
                    &settings,
                    &writer_path,
                    false,
                    move || {
                        writer_release.wait();
                        Ok(())
                    },
                    || Ok(()),
                    |_, _| Ok(()),
                    |_| Ok(()),
                )
            })
        };
        let left_writer = spawn_writer(left.clone());
        let right_writer = spawn_writer(right.clone());
        release.wait();

        left_writer.join().unwrap().unwrap();
        right_writer.join().unwrap().unwrap();
        observing.store(false, Ordering::Release);
        observer.join().unwrap();

        let final_settings = load_from_path(&path, false).unwrap();
        assert!(final_settings == left || final_settings == expected_right);
        let observed = observations.lock().unwrap();
        assert!(!observed.is_empty());
        for text in observed.iter() {
            let settings: ServerSettings = serde_json::from_str(text).unwrap();
            assert!(settings == initial || settings == left || settings == expected_right);
        }
        assert_eq!(partial_files(&dir), Vec::<String>::new());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn failed_publication_cleans_only_its_unique_partial() {
        let dir = temp_dir("failed-publication");
        let path = dir.join("server-settings.json");

        let error = write_atomically_with_before_publish(&path, b"{}", |_, _| {
            Err(std::io::Error::other("injected publication failure"))
        })
        .unwrap_err();

        assert!(error
            .to_string()
            .starts_with("Could not save server settings:"));
        assert!(error.to_string().contains("injected publication failure"));
        assert!(!path.exists());
        assert_eq!(partial_files(&dir), Vec::<String>::new());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_io_errors_are_reported_as_access_failures() {
        let dir = temp_dir("load-access-error");

        let error = load_from_path(&dir, false).unwrap_err();

        assert!(error
            .to_string()
            .starts_with("Could not access server settings:"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn locked_save_scavenges_only_matching_abandoned_unique_partials() {
        let dir = temp_dir("stale-unique");
        let path = dir.join("server-settings.json");
        let abandoned = dir.join("server-settings.json.424242.7.part");
        let unrelated = dir.join("server-settings.json.owner.part");
        std::fs::write(&abandoned, "abandoned").unwrap();
        std::fs::write(&unrelated, "keep").unwrap();

        save_to_path(&ServerSettings::default(), &path, false).unwrap();

        assert!(!abandoned.exists());
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "keep");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cross_process_schema2_publisher_helper() {
        let Ok(path) = std::env::var(CROSS_PROCESS_CHILD_PATH) else {
            return;
        };
        let ready = PathBuf::from(std::env::var(CROSS_PROCESS_READY_PATH).unwrap());
        let release = PathBuf::from(std::env::var(CROSS_PROCESS_RELEASE_PATH).unwrap());
        let path = PathBuf::from(path);
        let _lock = acquire_settings_lock(&path).unwrap();
        std::fs::write(&ready, b"locked").unwrap();
        assert!(wait_for_path(&release, Duration::from_secs(10)));
        write_atomically_locked_with_hooks(
            &path,
            CROSS_PROCESS_FUTURE.as_bytes(),
            |_, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap();
    }

    #[test]
    fn v1_writer_rechecks_schema_after_waiting_for_cross_process_lock() {
        let dir = temp_dir("cross-process-schema");
        let path = dir.join("server-settings.json");
        let ready = dir.join("child.ready");
        let release = dir.join("child.release");
        save_to_path(&ServerSettings::default(), &path, false).unwrap();

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "server_connector::config::tests::cross_process_schema2_publisher_helper",
                "--nocapture",
            ])
            .env(CROSS_PROCESS_CHILD_PATH, &path)
            .env(CROSS_PROCESS_READY_PATH, &ready)
            .env(CROSS_PROCESS_RELEASE_PATH, &release)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        if !wait_for_path(&ready, Duration::from_secs(10)) {
            child.kill().ok();
            child.wait().ok();
            panic!("schema2 child did not acquire the settings lock");
        }

        let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            save_to_path_with_hooks(
                &ServerSettings {
                    schema_version: CURRENT_SCHEMA_VERSION,
                    enabled: true,
                    base_url: Some("https://v1-writer.example".into()),
                },
                &writer_path,
                false,
                move || {
                    attempted_tx.send(()).unwrap();
                    Ok(())
                },
                move || {
                    acquired_tx.send(()).unwrap();
                    Ok(())
                },
                |_, _| Ok(()),
                |_| Ok(()),
            )
        });
        if attempted_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            child.kill().ok();
            child.wait().ok();
            panic!("v1 writer did not attempt the settings lock");
        }
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(200))
                .is_err(),
            "v1 writer acquired the lock while schema2 still held it"
        );
        std::fs::write(&release, b"publish").unwrap();

        let output = wait_for_child(child, Duration::from_secs(10));
        assert!(
            output.status.success(),
            "child stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        acquired_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            writer.join().unwrap(),
            Err(ConfigError::IncompatibleSchema(2))
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            CROSS_PROCESS_FUTURE
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn future_schema_is_preserved_and_reported_instead_of_downgraded() {
        let dir = temp_dir("future-schema");
        let path = dir.join("server-settings.json");
        let future = r#"{
  "schemaVersion": 2,
  "enabled": true,
  "baseUrl": "https://future.example",
  "futureField": "preserve-me"
}"#;
        std::fs::write(&path, future).unwrap();

        assert!(matches!(
            load_from_path(&path, false),
            Err(ConfigError::IncompatibleSchema(2))
        ));
        let replacement = ServerSettings {
            schema_version: CURRENT_SCHEMA_VERSION,
            enabled: false,
            base_url: None,
        };
        assert!(matches!(
            save_to_path(&replacement, &path, false),
            Err(ConfigError::IncompatibleSchema(2))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), future);
        assert_eq!(partial_files(&dir), Vec::<String>::new());

        std::fs::remove_dir_all(dir).ok();
    }
}
