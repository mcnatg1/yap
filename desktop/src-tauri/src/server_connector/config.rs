use std::path::{Path, PathBuf};

use reqwest::Url;

mod error;
mod persisted_file;
mod persistence;
mod platform;

pub use error::ConfigError;
use persisted_file::{read_persisted_bytes, read_persisted_text};
use persistence::{
    acquire_settings_access_lock, acquire_settings_lock, write_atomically_locked_with_hooks,
};

pub const CURRENT_SCHEMA_VERSION: u16 = 1;
const ORIGIN_APPROVAL_SCHEMA_VERSION: u16 = 1;
const MAX_SERVER_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerSettings {
    pub schema_version: u16,
    pub enabled: bool,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ServerOriginApproval {
    schema_version: u16,
    origin: String,
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

pub fn validate_base_url(raw: &str, allow_insecure_private: bool) -> Result<String, ConfigError> {
    if raw.len() > MAX_SERVER_URL_BYTES {
        return Err(ConfigError::Invalid("Server URL is too long."));
    }
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

pub(super) fn origin_is_approved(origin: &str) -> Result<bool, ConfigError> {
    origin_is_approved_at(
        &origin_approval_path(),
        origin,
        allow_insecure_private_server(),
    )
}

pub(super) fn approve_origin(origin: &str) -> Result<String, ConfigError> {
    approve_origin_at(
        &origin_approval_path(),
        origin,
        allow_insecure_private_server(),
    )
}

fn origin_is_approved_at(
    path: &Path,
    origin: &str,
    allow_insecure_private: bool,
) -> Result<bool, ConfigError> {
    let requested = validate_base_url(origin, allow_insecure_private)?;
    Ok(
        load_origin_approval_from_path(path, allow_insecure_private)?.as_deref()
            == Some(requested.as_str()),
    )
}

fn load_origin_approval_from_path(
    path: &Path,
    allow_insecure_private: bool,
) -> Result<Option<String>, ConfigError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(ConfigError::Invalid(
                "Server origin approval must be a regular file.",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigError::AccessIo(error)),
    }
    let _lock = acquire_settings_access_lock(path)?;
    let bytes = match read_persisted_bytes(path).map_err(ConfigError::AccessIo)? {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    let approval: ServerOriginApproval = serde_json::from_slice(&bytes)?;
    if approval.schema_version != ORIGIN_APPROVAL_SCHEMA_VERSION {
        return Err(ConfigError::Invalid(
            "Server origin approval uses an unsupported schema.",
        ));
    }
    let normalized = validate_base_url(&approval.origin, allow_insecure_private)?;
    if normalized != approval.origin {
        return Err(ConfigError::Invalid(
            "Server origin approval is not canonical.",
        ));
    }
    Ok(Some(normalized))
}

fn approve_origin_at(
    path: &Path,
    origin: &str,
    allow_insecure_private: bool,
) -> Result<String, ConfigError> {
    let origin = validate_base_url(origin, allow_insecure_private)?;
    let approval = ServerOriginApproval {
        schema_version: ORIGIN_APPROVAL_SCHEMA_VERSION,
        origin: origin.clone(),
    };
    let encoded = serde_json::to_vec_pretty(&approval)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(ConfigError::SaveIo)?;
    }
    let _lock = acquire_settings_lock(path)?;
    write_atomically_locked_with_hooks(path, &encoded, |_, _| Ok(()), |_| Ok(()))?;
    Ok(origin)
}

pub(crate) fn load_from_path(
    path: &Path,
    allow_insecure_private: bool,
) -> Result<ServerSettings, ConfigError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    match std::fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Ok(ServerSettings::default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ServerSettings::default());
        }
        Err(error) => return Err(ConfigError::AccessIo(error)),
    }
    let _lock = acquire_settings_access_lock(path)?;
    load_from_path_under_lock(path, allow_insecure_private)
}

fn load_from_path_under_lock(
    path: &Path,
    allow_insecure_private: bool,
) -> Result<ServerSettings, ConfigError> {
    let text = match read_persisted_text(path).map_err(ConfigError::AccessIo)? {
        Some(text) => text,
        None => return Ok(ServerSettings::default()),
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
    let text = match read_persisted_text(path).map_err(ConfigError::AccessIo)? {
        Some(text) => text,
        None => return Ok(()),
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

pub(super) fn normalize_settings(
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

fn origin_approval_path() -> PathBuf {
    crate::paths::app_data_dir().join("server-origin-approval.json")
}

fn allow_insecure_private_server() -> bool {
    std::env::var("YAP_ALLOW_INSECURE_PRIVATE_SERVER").as_deref() == Ok("1")
}

#[cfg(test)]
mod tests;
