use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeSessionToken(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct OwnerNamespace(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TrackId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    Dictation,
    Meeting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin {
    LiveCapture,
    ImportedFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerMode {
    PushToTalk,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    Microphone,
    SystemLoopback,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedTrackProvenance {
    Unknown,
    Mixed,
    UserDeclared(CaptureSource),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrackSource {
    Captured { source: CaptureSource },
    Imported { provenance: ImportedTrackProvenance },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTrackDescriptor {
    pub track_id: TrackId,
    pub source: TrackSource,
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub session_id: SessionId,
    pub mode: SessionMode,
    pub origin: SessionOrigin,
    pub trigger_mode: TriggerMode,
    pub started_at_utc: String,
    pub utc_offset_minutes_at_start: Option<i16>,
    pub locale_hint_bcp47: Option<String>,
    pub country_code_hint: Option<String>,
    pub preferred_languages_bcp47: Vec<String>,
    pub app_version: String,
    pub platform: String,
    pub privacy_policy_version: String,
    pub retention_expires_at_utc: Option<String>,
}

impl SessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_opaque_id(&value, 128, "session ID")?;
        Ok(Self(value))
    }

    pub fn generate() -> Result<Self, String> {
        let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self::generate_at(SystemTime::now(), std::process::id(), counter)
    }

    pub fn generate_at(time: SystemTime, process_id: u32, counter: u64) -> Result<Self, String> {
        let elapsed = time
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock is before the Unix epoch".to_string())?;
        let nanos = elapsed.as_nanos();
        Self::new(format!("s-{nanos:x}-{process_id:x}-{counter:x}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_current_writer_id(&self) -> bool {
        let mut parts = self.0.split('-');
        parts.next() == Some("s")
            && parts.next().is_some_and(is_generated_id_component)
            && parts.next().is_some_and(is_generated_id_component)
            && parts.next().is_some_and(is_generated_id_component)
            && parts.next().is_none()
    }
}

fn is_generated_id_component(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Allocates a durable recording identity by reserving its first artifact with
/// a caller-provided create-new operation. Existing artifact prefixes must be
/// reported as `AlreadyExists`, which advances the generated counter and retries.
pub fn allocate_recording<F>(mut reserve_artifact: F) -> Result<SessionId, String>
where
    F: FnMut(&SessionId) -> std::io::Result<()>,
{
    for _ in 0..1_024 {
        let session_id = SessionId::generate()?;
        match reserve_artifact(&session_id) {
            Ok(()) => return Ok(session_id),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("failed to reserve recording artifact: {error}")),
        }
    }
    Err("failed to allocate a collision-free recording session ID".into())
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for SessionId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl OwnerNamespace {
    pub fn local(install_id: impl AsRef<str>) -> Result<Self, String> {
        let install_id = install_id.as_ref();
        validate_opaque_id(install_id, 64, "install ID")?;
        Ok(Self(format!("local:{install_id}")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OwnerNamespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for OwnerNamespace {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let install_id = value
            .strip_prefix("local:")
            .ok_or_else(|| "owner namespace must use the local: prefix".to_string())?;
        Self::local(install_id)
    }
}

impl TrackId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        validate_opaque_id(&value, 64, "track ID")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<String> for TrackId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl CaptureTrackDescriptor {
    pub fn from_selector(
        track_id: TrackId,
        source: TrackSource,
        install_id: &str,
        selector_id: &str,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(install_id.as_bytes());
        hasher.update([0]);
        hasher.update(selector_id.as_bytes());
        let digest = hasher.finalize();
        let device_id = format!("dev-{}", hex_prefix(&digest, 16));

        Self {
            track_id,
            source,
            device_id,
        }
    }
}

impl SessionMetadata {
    pub fn dictation(started_at: SystemTime, trigger_mode: TriggerMode) -> Result<Self, String> {
        Self::dictation_with_hints(started_at, trigger_mode, None, Vec::new())
    }

    pub fn dictation_with_hints(
        started_at: SystemTime,
        trigger_mode: TriggerMode,
        country_code_hint: Option<String>,
        preferred_languages_bcp47: Vec<String>,
    ) -> Result<Self, String> {
        Self::new(
            SessionId::generate()?,
            SessionMode::Dictation,
            SessionOrigin::LiveCapture,
            trigger_mode,
            started_at,
            None,
            None,
            country_code_hint,
            preferred_languages_bcp47,
            None,
        )
    }

    pub fn meeting(
        started_at: SystemTime,
        trigger_mode: TriggerMode,
        retention_expires_at: Option<SystemTime>,
    ) -> Result<Self, String> {
        let retention_expires_at = retention_expires_at
            .unwrap_or_else(|| started_at + Duration::from_secs(30 * 24 * 60 * 60));
        Self::new(
            SessionId::generate()?,
            SessionMode::Meeting,
            SessionOrigin::LiveCapture,
            trigger_mode,
            started_at,
            None,
            None,
            None,
            Vec::new(),
            Some(retention_expires_at),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: SessionId,
        mode: SessionMode,
        origin: SessionOrigin,
        trigger_mode: TriggerMode,
        started_at: SystemTime,
        utc_offset_minutes_at_start: Option<i16>,
        locale_hint_bcp47: Option<String>,
        country_code_hint: Option<String>,
        preferred_languages_bcp47: Vec<String>,
        retention_expires_at: Option<SystemTime>,
    ) -> Result<Self, String> {
        if preferred_languages_bcp47.len() > 8 {
            return Err("at most eight preferred language hints are allowed".into());
        }
        if let Some(locale) = locale_hint_bcp47.as_deref() {
            validate_bcp47_hint(locale)?;
        }
        for language in &preferred_languages_bcp47 {
            validate_bcp47_hint(language)?;
        }
        let country_code_hint = country_code_hint
            .map(|country| normalize_country_code(&country))
            .transpose()?;
        if mode == SessionMode::Meeting && retention_expires_at.is_none() {
            return Err("meeting metadata requires a retention expiry".into());
        }
        if let Some(retention_expires_at) = retention_expires_at {
            if retention_expires_at <= started_at {
                return Err("retention expiry must be after session start".into());
            }
        }

        Ok(Self {
            session_id,
            mode,
            origin,
            trigger_mode,
            started_at_utc: format_utc(started_at)?,
            utc_offset_minutes_at_start,
            locale_hint_bcp47,
            country_code_hint,
            preferred_languages_bcp47,
            app_version: env!("CARGO_PKG_VERSION").into(),
            platform: std::env::consts::OS.into(),
            privacy_policy_version: "unconfigured".into(),
            retention_expires_at_utc: retention_expires_at.map(format_utc).transpose()?,
        })
    }
}

fn validate_opaque_id(value: &str, max_length: usize, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(format!(
            "{label} must contain 1-{max_length} ASCII letters, digits, _ or -"
        ));
    }
    Ok(())
}

fn validate_bcp47_hint(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 35
        || value.starts_with('-')
        || value.ends_with('-')
        || value.split('-').any(|subtag| {
            subtag.is_empty() || !subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        return Err("language hints must be BCP 47-like ASCII subtags up to 35 characters".into());
    }
    Ok(())
}

fn normalize_country_code(value: &str) -> Result<String, String> {
    if value.len() != 2 || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err("country hint must be exactly two ASCII letters".into());
    }
    Ok(value.to_ascii_uppercase())
}

fn format_utc(value: SystemTime) -> Result<String, String> {
    let elapsed = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "timestamp is before the Unix epoch".to_string())?;
    let nanos = i128::try_from(elapsed.as_nanos()).map_err(|_| "timestamp is out of range")?;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|_| "timestamp is out of range".to_string())?
        .format(&Rfc3339)
        .map_err(|_| "failed to format UTC timestamp".to_string())
}

fn hex_prefix(bytes: &[u8], length: usize) -> String {
    bytes
        .iter()
        .take(length)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        CaptureSource, ImportedTrackProvenance, SessionId, SessionMetadata, SessionMode,
        SessionOrigin, TrackId, TrackSource, TriggerMode,
    };
    use std::time::{Duration, SystemTime};

    #[test]
    fn track_id_rejects_empty_control_or_separator_values() {
        for value in ["", "track/name", "track:name", "track\nname", "track name"] {
            assert!(TrackId::new(value).is_err(), "{value:?} should be rejected");
        }
        assert_eq!(TrackId::new("mic_1-A").unwrap().as_str(), "mic_1-A");
    }

    #[test]
    fn session_mode_origin_trigger_and_source_round_trip_independently() {
        let cases = [
            (SessionMode::Dictation, "dictation"),
            (SessionMode::Meeting, "meeting"),
        ];
        for (value, serialized) in cases {
            assert_eq!(serde_json::to_value(value).unwrap(), serialized);
            assert_eq!(
                serde_json::from_value::<SessionMode>(serialized.into()).unwrap(),
                value
            );
        }
        assert_eq!(
            serde_json::from_value::<SessionOrigin>("imported_file".into()).unwrap(),
            SessionOrigin::ImportedFile
        );
        assert_eq!(
            serde_json::from_value::<TriggerMode>("toggle".into()).unwrap(),
            TriggerMode::Toggle
        );
        assert_eq!(
            serde_json::from_value::<CaptureSource>("microphone".into()).unwrap(),
            CaptureSource::Microphone
        );
    }

    #[test]
    fn generated_session_ids_remain_distinct_across_process_and_counter_inputs() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(17);
        let one = SessionId::generate_at(at, 1, 1).unwrap();
        let other_process = SessionId::generate_at(at, 2, 1).unwrap();
        let other_counter = SessionId::generate_at(at, 1, 2).unwrap();

        assert_ne!(one, other_process);
        assert_ne!(one, other_counter);
    }

    #[test]
    fn recording_allocation_retries_create_new_collisions() {
        let mut attempts = 0;
        let session_id = super::allocate_recording(|_| {
            attempts += 1;
            if attempts == 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "collision",
                ));
            }
            Ok(())
        })
        .unwrap();

        assert_eq!(attempts, 2);
        assert!(session_id.as_str().starts_with("s-"));
    }

    #[test]
    fn imported_origin_does_not_claim_a_physical_capture_source() {
        let track = TrackSource::Imported {
            provenance: ImportedTrackProvenance::Unknown,
        };
        let value = serde_json::to_value(track).unwrap();

        assert_eq!(value["kind"], "imported");
        assert!(value.get("source").is_none());
    }

    #[test]
    fn metadata_formats_utc_as_rfc3339_and_keeps_timing_monotonic_elsewhere() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let metadata = SessionMetadata::dictation(start, TriggerMode::PushToTalk).unwrap();

        assert!(metadata.started_at_utc.ends_with('Z'));
        assert!(metadata.retention_expires_at_utc.is_none());
        assert_eq!(
            SessionMetadata::meeting(start, TriggerMode::Toggle, None)
                .unwrap()
                .retention_expires_at_utc,
            Some("2023-12-14T22:13:20Z".into())
        );
    }

    #[test]
    fn metadata_bounds_language_hints_and_validates_country_without_location_inference() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let languages = vec!["en-US".to_string(); 8];
        let metadata = SessionMetadata::dictation_with_hints(
            start,
            TriggerMode::PushToTalk,
            Some("us".into()),
            languages,
        )
        .unwrap();

        assert_eq!(metadata.country_code_hint.as_deref(), Some("US"));
        assert!(SessionMetadata::dictation_with_hints(
            start,
            TriggerMode::PushToTalk,
            None,
            vec!["en".into(); 9],
        )
        .is_err());
        assert!(SessionMetadata::dictation_with_hints(
            start,
            TriggerMode::PushToTalk,
            Some("USA".into()),
            Vec::new(),
        )
        .is_err());
    }

    #[test]
    fn meeting_metadata_requires_an_explicit_retention_expiry() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let defaulted = SessionMetadata::meeting(start, TriggerMode::Toggle, None).unwrap();
        let explicit = SessionMetadata::meeting(
            start,
            TriggerMode::Toggle,
            Some(start + Duration::from_secs(7 * 24 * 60 * 60)),
        )
        .unwrap();

        assert!(defaulted.retention_expires_at_utc.is_some());
        assert!(explicit.retention_expires_at_utc.is_some());
    }
}
