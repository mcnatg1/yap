mod admission;
mod http;
mod server;
mod source;

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use admission::{token_from_url, MediaEntry, MediaOwnerInner};
use server::MediaServer;
use source::{authorize_playback_source, AuthorizedMediaSource};

pub(crate) use source::{
    inspect_media_source, open_unchanged_media_source, MediaSourceFingerprint,
};

// Tauri 2.11 custom protocol responders require a complete Cow<'static, [u8]>
// body. A loopback owner preserves HTTP range semantics without buffering media.
const DEFAULT_MAX_ACTIVE_ADMISSIONS: usize = 1024;
const DEFAULT_ADMISSION_IDLE_TTL: Duration = Duration::from_secs(30 * 60);
const DEFAULT_ADMISSION_MAX_TTL: Duration = Duration::from_secs(4 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaAdmission {
    pub(crate) byte_length: String,
    pub(crate) url: String,
    pub(crate) waveform_eligible: bool,
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
        Self::with_policy(
            capacity,
            DEFAULT_ADMISSION_IDLE_TTL,
            DEFAULT_ADMISSION_MAX_TTL,
            Arc::new(Instant::now),
        )
    }

    fn with_policy(
        capacity: usize,
        idle_ttl: Duration,
        max_ttl: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(MediaOwnerInner::new(capacity, idle_ttl, max_ttl, clock)),
            server: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn with_capacity_for_test(capacity: usize) -> Self {
        Self::with_capacity(capacity)
    }

    #[cfg(test)]
    fn with_policy_for_test(
        capacity: usize,
        idle_ttl: Duration,
        max_ttl: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        Self::with_policy(capacity, idle_ttl, max_ttl, clock)
    }

    pub(crate) fn admit(
        &self,
        path: &Path,
        waveform_byte_limit: u64,
    ) -> Result<MediaAdmission, String> {
        let source = authorize_playback_source(path, None)?;
        self.admit_authorized(source, waveform_byte_limit)
    }

    pub(crate) fn admit_unchanged(
        &self,
        path: &Path,
        expected: &MediaSourceFingerprint,
        waveform_byte_limit: u64,
    ) -> Result<MediaAdmission, String> {
        let source = authorize_playback_source(path, Some(expected))?;
        self.admit_authorized(source, waveform_byte_limit)
    }

    fn admit_authorized(
        &self,
        source: AuthorizedMediaSource,
        waveform_byte_limit: u64,
    ) -> Result<MediaAdmission, String> {
        let authority = self.ensure_server()?;
        let byte_length = source.byte_length();
        let token = self
            .inner
            .insert_admission(MediaEntry::new(source, self.inner.now()))?;
        let (byte_length, waveform_eligible) = admission_metadata(byte_length, waveform_byte_limit);
        Ok(MediaAdmission {
            byte_length,
            url: format!("http://{authority}/media/{token}"),
            waveform_eligible,
        })
    }

    pub(crate) fn release(&self, url: &str) -> bool {
        let authority = match self.server.lock() {
            Ok(server) => server.as_ref().map(|server| server.authority().to_string()),
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
            .authority()
            .to_string())
    }

    #[cfg(test)]
    pub(crate) fn active_admission_count_for_test(&self) -> usize {
        self.inner.active_admission_count()
    }
}

fn admission_metadata(length: u64, waveform_byte_limit: u64) -> (String, bool) {
    (length.to_string(), length <= waveform_byte_limit)
}

#[cfg(test)]
mod tests;
