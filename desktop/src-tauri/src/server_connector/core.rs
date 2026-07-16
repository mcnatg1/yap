use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use crate::runtime;

use super::{
    batch, client, config,
    state::{self, ConnectorInner, SettingsDisposition},
    ServerConnectionSnapshot,
};

pub struct ServerConnector {
    pub(super) client: reqwest::Client,
    pub(super) inner: Mutex<ConnectorInner>,
    pub(super) generation: AtomicU64,
    settings_save_active: Arc<AtomicBool>,
}

#[derive(Debug)]
pub(super) struct SettingsSaveLease {
    active: Arc<AtomicBool>,
}

pub(crate) struct BatchConnectionLease {
    generation: u64,
    base_url: String,
    client: batch::BatchApiClient,
}

impl BatchConnectionLease {
    pub(crate) fn client(&self) -> &batch::BatchApiClient {
        &self.client
    }
}

impl Default for ServerConnector {
    fn default() -> Self {
        Self {
            client: client::bounded_client().expect("bounded server connector client must build"),
            inner: Mutex::new(ConnectorInner::default()),
            generation: AtomicU64::new(0),
            settings_save_active: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ServerConnector {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn begin_settings_save(&self) -> Result<SettingsSaveLease, String> {
        self.settings_save_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "A server settings update is already active.".to_string())?;
        Ok(SettingsSaveLease {
            active: Arc::clone(&self.settings_save_active),
        })
    }

    #[cfg(test)]
    pub(crate) fn current(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn invalidate(&self) -> u64 {
        let mut inner = self.inner.lock().expect("server connector poisoned");
        self.invalidate_locked(&mut inner)
    }

    pub(super) fn invalidate_locked(&self, inner: &mut ConnectorInner) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        inner.apply_settings(generation, SettingsDisposition::NotSet);
        generation
    }

    pub(super) fn with_loaded_settings<T, Load, Apply>(
        &self,
        load: Load,
        apply: Apply,
    ) -> Result<T, config::ConfigError>
    where
        Load: FnOnce() -> Result<config::ServerSettings, config::ConfigError>,
        Apply: FnOnce(&mut ConnectorInner, config::ServerSettings) -> T,
    {
        let mut inner = self.inner.lock().expect("server connector poisoned");
        let settings = load()?;
        Ok(apply(&mut inner, settings))
    }

    pub(super) fn synchronize_settings_with<Project>(
        &self,
        settings: &config::ServerSettings,
        project: Project,
    ) -> ServerConnectionSnapshot
    where
        Project: Fn(&ServerConnectionSnapshot),
    {
        let mut inner = self.inner.lock().expect("server connector poisoned");
        self.synchronize_settings_locked(&mut inner, settings, project)
    }

    pub(super) fn synchronize_settings_locked<Project>(
        &self,
        inner: &mut ConnectorInner,
        settings: &config::ServerSettings,
        project: Project,
    ) -> ServerConnectionSnapshot
    where
        Project: Fn(&ServerConnectionSnapshot),
    {
        let mut generation = self.generation.load(Ordering::Acquire);
        if !inner.configuration_matches(generation, settings.enabled, settings.base_url.as_deref())
        {
            if inner.generation() == generation && inner.current_configuration_initialized() {
                generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
            }
            inner.apply_server_settings(generation, settings.enabled, settings.base_url.clone());
            project(&inner.snapshot());
        }
        inner.snapshot()
    }

    pub(super) fn snapshot(&self) -> ServerConnectionSnapshot {
        self.inner
            .lock()
            .expect("server connector poisoned")
            .snapshot()
    }

    pub(crate) fn batch_connection_lease(&self) -> Result<Option<BatchConnectionLease>, String> {
        let generation = self.generation.load(Ordering::Acquire);
        let inner = self.inner.lock().expect("server connector poisoned");
        let snapshot = inner.snapshot();
        if inner.generation() != generation
            || snapshot.state != runtime::state::ServerConnectorState::Ready
            || !snapshot.capabilities.batch_jobs
            || !snapshot.capabilities.job_status
        {
            return Ok(None);
        }
        let Some(base_url) = inner.configured_base_url(generation) else {
            return Ok(None);
        };
        let client = batch::BatchApiClient::new(self.client.clone(), &base_url)
            .map_err(|error| error.to_string())?;
        let base_url = client.base_url_identity().to_owned();
        Ok(Some(BatchConnectionLease {
            generation,
            base_url,
            client,
        }))
    }

    pub(crate) fn persisted_cleanup_client(
        &self,
        base_url: &str,
    ) -> Result<batch::BatchApiClient, String> {
        // A durable cancellation record is a cleanup-only authority for its
        // exact previously validated origin. It does not authorize new work.
        batch::BatchApiClient::new(self.client.clone(), base_url).map_err(|error| error.to_string())
    }

    pub(crate) fn configured_batch_origin(&self) -> Result<Option<String>, String> {
        let generation = self.generation.load(Ordering::Acquire);
        let inner = self.inner.lock().expect("server connector poisoned");
        if inner.generation() != generation || !inner.current_configuration_initialized() {
            return Err("Server settings are not initialized for remote cleanup.".into());
        }
        Ok(inner.configured_base_url(generation))
    }

    pub(crate) fn with_current_batch_lease<T>(
        &self,
        lease: &BatchConnectionLease,
        commit: impl FnOnce() -> T,
    ) -> Result<T, String> {
        let inner = self.inner.lock().expect("server connector poisoned");
        let snapshot = inner.snapshot();
        let current = self.generation.load(Ordering::Acquire) == lease.generation
            && inner.generation() == lease.generation
            && inner.configured_base_url(lease.generation).as_deref()
                == Some(lease.base_url.as_str())
            && snapshot.state == runtime::state::ServerConnectorState::Ready
            && snapshot.capabilities.batch_jobs
            && snapshot.capabilities.job_status;
        if !current {
            return Err("Server connection changed before the batch response could commit.".into());
        }
        Ok(commit())
    }

    pub(super) fn begin_health_request_with<Project>(
        &self,
        project: Project,
    ) -> Option<(u64, String)>
    where
        Project: Fn(&ServerConnectionSnapshot),
    {
        let generation = self.generation.load(Ordering::Acquire);
        let mut inner = self.inner.lock().expect("server connector poisoned");
        let base_url = inner.configured_base_url(generation)?;
        if !inner.begin_health_request(generation, now_ms()) {
            return None;
        }
        project(&inner.snapshot());
        Some((generation, base_url))
    }

    pub(super) fn accept_health_result_with<Project, SpawnRetry>(
        &self,
        generation: u64,
        result: client::HealthCheckResult,
        project: Project,
        spawn_retry_task: SpawnRetry,
    ) -> ServerConnectionSnapshot
    where
        Project: Fn(&ServerConnectionSnapshot),
        SpawnRetry: FnOnce(u64, u64, Duration) -> tauri::async_runtime::JoinHandle<()>,
    {
        {
            let mut inner = self.inner.lock().expect("server connector poisoned");
            if self.generation.load(Ordering::Acquire) != generation {
                return inner.snapshot();
            }
            let Some(transition) =
                inner.finish_health_request(generation, result, now_ms(), state::production_jitter)
            else {
                return inner.snapshot();
            };
            project(&inner.snapshot());

            if let Some(delay) = transition.retry_after {
                let retry_at_ms = now_ms().saturating_add(duration_ms(delay));
                if inner.arm_retry(generation, retry_at_ms) {
                    let snapshot = inner.snapshot();
                    project(&snapshot);
                    let retry_token = inner.retry_token();
                    let task = spawn_retry_task(generation, retry_token, delay);
                    inner.install_retry_task(task);
                }
            }
        }

        self.snapshot()
    }

    pub(super) fn begin_scheduled_retry_with<Project>(
        &self,
        generation: u64,
        retry_token: u64,
        project: Project,
    ) -> Option<String>
    where
        Project: Fn(&ServerConnectionSnapshot),
    {
        let mut inner = self.inner.lock().expect("server connector poisoned");
        if self.generation.load(Ordering::Acquire) != generation
            || !inner.begin_scheduled_retry(generation, retry_token)
        {
            return None;
        }
        let base_url = inner.configured_base_url(generation)?;
        project(&inner.snapshot());
        Some(base_url)
    }
}

impl Drop for SettingsSaveLease {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
