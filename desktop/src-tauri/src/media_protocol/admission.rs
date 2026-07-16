use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use super::source::AuthorizedMediaSource;

const TOKEN_BYTES: usize = 32;
pub(super) const TOKEN_HEX_LENGTH: usize = TOKEN_BYTES * 2;

pub(super) struct MediaOwnerInner {
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    idle_ttl: Duration,
    max_ttl: Duration,
    registry: Mutex<MediaRegistry>,
}

impl MediaOwnerInner {
    pub(super) fn new(
        capacity: usize,
        idle_ttl: Duration,
        max_ttl: Duration,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        Self {
            clock,
            idle_ttl,
            max_ttl,
            registry: Mutex::new(MediaRegistry {
                capacity: capacity.max(1),
                entries: HashMap::new(),
                order: VecDeque::new(),
            }),
        }
    }

    pub(super) fn now(&self) -> Instant {
        (self.clock)()
    }

    pub(super) fn insert_admission(&self, entry: MediaEntry) -> Result<String, String> {
        let mut registry = self
            .registry
            .lock()
            .map_err(|_| "Media registry lock is unavailable.".to_string())?;
        registry.prune_expired(self.now(), self.idle_ttl, self.max_ttl);
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

    pub(super) fn admission(&self, token: &str) -> Option<Arc<MediaEntry>> {
        let now = self.now();
        let mut registry = self.registry.lock().ok()?;
        let entry = registry.entries.get(token).cloned()?;
        if entry.is_expired(now, self.idle_ttl, self.max_ttl) {
            registry.remove(token);
            return None;
        }
        entry.touch(now)?;
        Some(entry)
    }

    pub(super) fn is_current(&self, token: &str, expected: &Arc<MediaEntry>) -> bool {
        let now = self.now();
        let Ok(mut registry) = self.registry.lock() else {
            return false;
        };
        if expected.is_expired(now, self.idle_ttl, self.max_ttl) {
            registry.remove(token);
            return false;
        }
        registry
            .entries
            .get(token)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
            && !expected.revoked.load(Ordering::Acquire)
    }

    pub(super) fn revoke(&self, token: &str) -> bool {
        let Ok(mut registry) = self.registry.lock() else {
            return false;
        };
        registry.remove(token).is_some()
    }

    pub(super) fn revoke_if_current(&self, token: &str, expected: &Arc<MediaEntry>) {
        let Ok(mut registry) = self.registry.lock() else {
            expected.revoked.store(true, Ordering::Release);
            return;
        };
        if registry
            .entries
            .get(token)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            registry.remove(token);
        }
        expected.revoked.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(super) fn active_admission_count(&self) -> usize {
        let Ok(mut registry) = self.registry.lock() else {
            return 0;
        };
        registry.prune_expired(self.now(), self.idle_ttl, self.max_ttl);
        registry.entries.len()
    }
}

struct MediaRegistry {
    capacity: usize,
    entries: HashMap<String, Arc<MediaEntry>>,
    order: VecDeque<String>,
}

impl MediaRegistry {
    fn remove(&mut self, token: &str) -> Option<Arc<MediaEntry>> {
        let entry = self.entries.remove(token)?;
        self.order.retain(|candidate| candidate != token);
        entry.revoked.store(true, Ordering::Release);
        Some(entry)
    }

    fn prune_expired(&mut self, now: Instant, idle_ttl: Duration, max_ttl: Duration) {
        let expired = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.is_expired(now, idle_ttl, max_ttl))
            .map(|(token, _)| token.clone())
            .collect::<Vec<_>>();
        for token in expired {
            self.remove(&token);
        }
    }
}

pub(super) struct MediaEntry {
    created_at: Instant,
    last_used: Mutex<Instant>,
    revoked: Arc<AtomicBool>,
    source: AuthorizedMediaSource,
}

impl MediaEntry {
    pub(super) fn new(source: AuthorizedMediaSource, now: Instant) -> Self {
        Self {
            created_at: now,
            last_used: Mutex::new(now),
            revoked: Arc::new(AtomicBool::new(false)),
            source,
        }
    }

    pub(super) fn byte_length(&self) -> u64 {
        self.source.byte_length()
    }

    pub(super) fn mime(&self) -> &'static str {
        self.source.mime
    }

    pub(super) fn revoked(&self) -> &Arc<AtomicBool> {
        &self.revoked
    }

    pub(super) fn source_file(&self) -> &File {
        &self.source.file
    }

    pub(super) fn source_is_unchanged(&self) -> bool {
        self.source.is_unchanged()
    }

    fn is_expired(&self, now: Instant, idle_ttl: Duration, max_ttl: Duration) -> bool {
        let Ok(last_used) = self.last_used.lock() else {
            return true;
        };
        now.saturating_duration_since(self.created_at) >= max_ttl
            || now.saturating_duration_since(*last_used) >= idle_ttl
    }

    fn touch(&self, now: Instant) -> Option<()> {
        *self.last_used.lock().ok()? = now;
        Some(())
    }
}

pub(super) fn token_from_url(url: &str, authority: &str) -> Option<String> {
    let path = url.strip_prefix(&format!("http://{authority}"))?;
    token_from_request_path(path).map(str::to_string)
}

pub(super) fn token_from_request_path(path: &str) -> Option<&str> {
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
    use std::io::Read;

    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .map_err(|error| format!("Failed to mint media admission token: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn fill_secure_random(_bytes: &mut [u8]) -> Result<(), String> {
    Err("Secure media admission tokens are unsupported on this platform.".into())
}
