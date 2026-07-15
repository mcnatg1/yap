pub(crate) mod batch;
mod client;
pub mod config;
mod state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tauri::{Emitter, Manager};

use crate::runtime;
use state::{ConnectorInner, SettingsDisposition};

pub use state::{ServerCapabilities, ServerConnectionSnapshot};

pub struct ServerConnector {
    client: reqwest::Client,
    inner: Mutex<ConnectorInner>,
    generation: AtomicU64,
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

/// App-independent adapter over the production connector boundary.
///
/// This is intentionally narrow: integration tests and non-Tauri hosts can
/// drive the same bounded HTTP client, state machine, generation checks, and
/// retry cancellation used by the desktop command adapter without exposing
/// those implementation modules.
#[doc(hidden)]
#[derive(Clone)]
pub struct ServerConnectorBoundary {
    connector: Arc<ServerConnector>,
}

impl Default for ServerConnectorBoundary {
    fn default() -> Self {
        Self {
            connector: Arc::new(ServerConnector::default()),
        }
    }
}

impl ServerConnectorBoundary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn configure(&self, settings: &config::ServerSettings) -> ServerConnectionSnapshot {
        self.connector.synchronize_settings_with(settings, |_| {})
    }

    pub fn snapshot(&self) -> ServerConnectionSnapshot {
        self.connector.snapshot()
    }

    #[doc(hidden)]
    pub fn downgrade(&self) -> Weak<ServerConnector> {
        Arc::downgrade(&self.connector)
    }

    pub async fn refresh(&self) -> ServerConnectionSnapshot {
        let Some((generation, base_url)) = self.connector.begin_health_request_with(|_| {}) else {
            return self.snapshot();
        };

        let result = client::check_health(
            &self.connector.client,
            &base_url,
            allow_insecure_private_server(),
        )
        .await;
        let retry_connector = Arc::downgrade(&self.connector);
        self.connector.accept_health_result_with(
            generation,
            result,
            |_| {},
            move |generation, retry_token, delay| {
                spawn_boundary_retry(retry_connector, generation, retry_token, delay)
            },
        )
    }
}

impl Default for ServerConnector {
    fn default() -> Self {
        Self {
            client: client::bounded_client().expect("bounded server connector client must build"),
            inner: Mutex::new(ConnectorInner::default()),
            generation: AtomicU64::new(0),
        }
    }
}

impl ServerConnector {
    pub fn new() -> Self {
        Self::default()
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

    fn invalidate_locked(&self, inner: &mut ConnectorInner) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        inner.apply_settings(generation, SettingsDisposition::NotSet);
        generation
    }

    fn with_loaded_settings<T, Load, Apply>(
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

    fn synchronize_from_disk(
        &self,
        app: &tauri::AppHandle,
    ) -> Result<ServerConnectionSnapshot, config::ConfigError> {
        self.with_loaded_settings(config::load, |inner, settings| {
            self.synchronize_settings_locked(inner, &settings, |snapshot| {
                emit_transition(app, snapshot);
            })
        })
    }

    fn synchronize_settings_with<Project>(
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

    fn synchronize_settings_locked<Project>(
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

    fn snapshot(&self) -> ServerConnectionSnapshot {
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
        // exact previously validated origin. It does not authorize new work
        // and deliberately does not depend on the current settings generation.
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

    pub(crate) async fn refresh_for_job_drain(
        &self,
        app: &tauri::AppHandle,
    ) -> ServerConnectionSnapshot {
        if self.synchronize_from_disk(app).is_err() {
            return self.snapshot();
        }
        self.refresh(app).await
    }

    async fn refresh<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> ServerConnectionSnapshot {
        let Some((generation, base_url)) = self.begin_health_request_with(|snapshot| {
            emit_transition(app, snapshot);
        }) else {
            return self.snapshot();
        };

        let result = check_health_for_approved_origin(
            &self.client,
            &base_url,
            allow_insecure_private_server(),
            config::origin_is_approved,
        )
        .await;
        let retry_app = app.clone();
        self.accept_health_result_with(
            generation,
            result,
            |snapshot| emit_transition(app, snapshot),
            move |generation, retry_token, delay| {
                spawn_retry(retry_app, generation, retry_token, delay)
            },
        )
    }

    fn begin_health_request_with<Project>(&self, project: Project) -> Option<(u64, String)>
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

    fn accept_health_result_with<Project, SpawnRetry>(
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

    fn begin_scheduled_retry_with<Project>(
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

fn emit_transition<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    snapshot: &ServerConnectionSnapshot,
) {
    if let Err(error) = app.emit_to(
        crate::authorization::MAIN_WINDOW_LABEL,
        "server-connection",
        snapshot.clone(),
    ) {
        crate::stt::log_yap(&format!("server connection event failed: {error}"));
    }
}

async fn check_health_for_approved_origin<Authorize>(
    client: &reqwest::Client,
    base_url: &str,
    allow_insecure_private: bool,
    authorize: Authorize,
) -> client::HealthCheckResult
where
    Authorize: FnOnce(&str) -> Result<bool, config::ConfigError>,
{
    if !authorize(base_url).unwrap_or(false) {
        return client::HealthCheckResult::Offline {
            api_version: None,
            error_code: "UNAPPROVED_SERVER_ORIGIN",
            retryable: false,
        };
    }
    client::check_health(client, base_url, allow_insecure_private).await
}

fn spawn_retry<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    generation: u64,
    retry_token: u64,
    delay: Duration,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        Box::pin(run_scheduled_retry(app, generation, retry_token)).await;
    })
}

async fn run_scheduled_retry<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    generation: u64,
    retry_token: u64,
) {
    let connector = app.state::<ServerConnector>();
    let Some(base_url) =
        connector.begin_scheduled_retry_with(generation, retry_token, |snapshot| {
            emit_transition(&app, snapshot);
        })
    else {
        return;
    };

    let result = check_health_for_approved_origin(
        &connector.client,
        &base_url,
        allow_insecure_private_server(),
        config::origin_is_approved,
    )
    .await;
    let retry_app = app.clone();
    connector.accept_health_result_with(
        generation,
        result,
        |snapshot| emit_transition(&app, snapshot),
        move |generation, retry_token, delay| {
            spawn_retry(retry_app, generation, retry_token, delay)
        },
    );
}

fn spawn_boundary_retry(
    connector: Weak<ServerConnector>,
    generation: u64,
    retry_token: u64,
    delay: Duration,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        let Some(connector) = connector.upgrade() else {
            return;
        };
        Box::pin(run_boundary_scheduled_retry(
            connector,
            generation,
            retry_token,
        ))
        .await;
    })
}

async fn run_boundary_scheduled_retry(
    connector: Arc<ServerConnector>,
    generation: u64,
    retry_token: u64,
) {
    let Some(base_url) = connector.begin_scheduled_retry_with(generation, retry_token, |_| {})
    else {
        return;
    };
    let result = client::check_health(
        &connector.client,
        &base_url,
        allow_insecure_private_server(),
    )
    .await;
    let retry_connector = Arc::downgrade(&connector);
    connector.accept_health_result_with(
        generation,
        result,
        |_| {},
        move |generation, retry_token, delay| {
            spawn_boundary_retry(retry_connector, generation, retry_token, delay)
        },
    );
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

fn allow_insecure_private_server() -> bool {
    std::env::var("YAP_ALLOW_INSECURE_PRIVATE_SERVER").as_deref() == Ok("1")
}

#[tauri::command]
pub(crate) fn server_connection_status(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
) -> Result<ServerConnectionSnapshot, String> {
    crate::authorization::ensure_main(&window)?;
    connector
        .synchronize_from_disk(&app)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn refresh_server_connection(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
) -> Result<ServerConnectionSnapshot, String> {
    crate::authorization::ensure_main(&window)?;
    connector
        .synchronize_from_disk(&app)
        .map_err(|error| error.to_string())?;
    Ok(connector.refresh(&app).await)
}

#[tauri::command]
pub(crate) fn server_settings(
    window: tauri::WebviewWindow,
) -> Result<config::ServerSettings, String> {
    crate::authorization::ensure_main(&window)?;
    config::load().map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn set_server_settings(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
    settings: config::ServerSettings,
) -> Result<config::ServerSettings, String> {
    crate::authorization::ensure_main(&window)?;
    let normalized = config::normalize_settings(&settings, allow_insecure_private_server())
        .map_err(|error| error.to_string())?;
    let current = config::load().map_err(|error| error.to_string())?;
    let origin_is_approved = normalized
        .base_url
        .as_deref()
        .is_some_and(|origin| config::origin_is_approved(origin).unwrap_or(false));
    let approval_origin =
        if requires_server_origin_confirmation(&current, &normalized, origin_is_approved) {
            let origin = normalized
                .base_url
                .clone()
                .expect("enabled normalized server settings have an origin");
            if !confirm_server_origin(app.clone(), origin.clone()).await? {
                return Err("Server connection change was cancelled.".into());
            }
            Some(origin)
        } else {
            None
        };

    let mut inner = connector.inner.lock().expect("server connector poisoned");
    let generation = connector.invalidate_locked(&mut inner);

    // Revoke the old lease before either durable setting changes or approval
    // publication. The candidate is saved first so an approval-write failure
    // leaves the new origin configured but unauthorized, which fails closed.
    let save_result = config::save(&normalized).and_then(|saved| {
        if let Some(origin) = approval_origin.as_deref() {
            config::approve_origin(origin)?;
        }
        Ok(saved)
    });
    let result = finish_settings_save_after_revocation(save_result);
    let effective = result
        .as_ref()
        .ok()
        .cloned()
        .or_else(|| config::load().ok())
        .unwrap_or(current);
    inner.apply_server_settings(generation, effective.enabled, effective.base_url.clone());
    emit_transition(&app, &inner.snapshot());
    result
}

fn requires_server_origin_confirmation(
    current: &config::ServerSettings,
    candidate: &config::ServerSettings,
    origin_is_approved: bool,
) -> bool {
    candidate.enabled
        && (!origin_is_approved
            || !current.enabled
            || current.base_url.as_deref() != candidate.base_url.as_deref())
}

async fn confirm_server_origin(app: tauri::AppHandle, origin: String) -> Result<bool, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(format!(
                "Allow Yap to connect to this private server?\n\n{origin}\n\nOnly approve an address supplied by your trusted administrator."
            ))
            .title("Confirm private server")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Connect".into(),
                "Cancel".into(),
            ))
            .blocking_show()
    })
    .await
    .map_err(|error| format!("Could not show server confirmation: {error}"))
}

#[cfg(test)]
fn finish_settings_save(
    connector: &ServerConnector,
    result: Result<config::ServerSettings, config::ConfigError>,
) -> Result<config::ServerSettings, String> {
    let mut inner = connector.inner.lock().expect("server connector poisoned");
    connector.invalidate_locked(&mut inner);
    finish_settings_save_after_revocation(result)
}

fn finish_settings_save_after_revocation(
    result: Result<config::ServerSettings, config::ConfigError>,
) -> Result<config::ServerSettings, String> {
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_batch_connection_lease_cannot_commit_after_configuration_changes() {
        let connector = ServerConnector::default();
        connector.synchronize_settings_with(
            &config::ServerSettings {
                schema_version: config::CURRENT_SCHEMA_VERSION,
                enabled: true,
                base_url: Some("http://127.0.0.1:18765".into()),
            },
            |_| {},
        );
        let (generation, _) = connector
            .begin_health_request_with(|_| {})
            .expect("configured connector begins health request");
        connector.accept_health_result_with(
            generation,
            client::HealthCheckResult::Ready {
                api_version: "1".into(),
                capabilities: ServerCapabilities {
                    batch_jobs: true,
                    live_streaming: false,
                    job_status: true,
                },
            },
            |_| {},
            |_, _, _| tauri::async_runtime::spawn(async {}),
        );
        let lease = connector
            .batch_connection_lease()
            .unwrap()
            .expect("ready batch-capable connector yields a lease");
        connector.invalidate();

        let committed = std::sync::atomic::AtomicBool::new(false);
        assert!(connector
            .with_current_batch_lease(&lease, || {
                committed.store(true, Ordering::SeqCst);
            })
            .is_err());
        assert!(!committed.load(Ordering::SeqCst));
    }

    #[test]
    fn new_or_reenabled_server_origins_require_native_confirmation() {
        let disabled = config::ServerSettings {
            schema_version: config::CURRENT_SCHEMA_VERSION,
            enabled: false,
            base_url: Some("https://asr.example.test/v1".into()),
        };
        let enabled = config::ServerSettings {
            enabled: true,
            ..disabled.clone()
        };
        assert!(requires_server_origin_confirmation(
            &disabled, &enabled, false
        ));
        assert!(requires_server_origin_confirmation(
            &enabled, &enabled, false
        ));
        assert!(!requires_server_origin_confirmation(
            &enabled, &enabled, true
        ));

        let changed = config::ServerSettings {
            base_url: Some("https://other.example.test/v1".into()),
            ..enabled.clone()
        };
        assert!(requires_server_origin_confirmation(
            &enabled, &changed, false
        ));

        let disabled_change = config::ServerSettings {
            enabled: false,
            ..changed
        };
        assert!(!requires_server_origin_confirmation(
            &enabled,
            &disabled_change,
            false
        ));
    }

    #[test]
    fn unapproved_origin_fails_before_any_health_socket_is_created() {
        let result = tauri::async_runtime::block_on(check_health_for_approved_origin(
            &client::bounded_client().unwrap(),
            "http://127.0.0.1:9",
            false,
            |_| Ok(false),
        ));

        assert_eq!(
            result,
            client::HealthCheckResult::Offline {
                api_version: None,
                error_code: "UNAPPROVED_SERVER_ORIGIN",
                retryable: false,
            }
        );
    }

    #[test]
    fn settings_load_cannot_run_ahead_of_the_connector_save_lock() {
        use std::sync::{mpsc, Arc};

        let connector = Arc::new(ServerConnector::default());
        let save_guard = connector.inner.lock().unwrap();
        let (load_started_tx, load_started_rx) = mpsc::channel();
        let waiting_connector = Arc::clone(&connector);
        let waiter = std::thread::spawn(move || {
            waiting_connector
                .with_loaded_settings(
                    || {
                        load_started_tx.send(()).unwrap();
                        Ok(config::ServerSettings::default())
                    },
                    |_, _| (),
                )
                .unwrap();
        });

        assert!(load_started_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());
        drop(save_guard);
        load_started_rx.recv().unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn delayed_health_response_cannot_mutate_a_new_settings_generation() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{mpsc, Arc};

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (request_started_tx, request_started_rx) = mpsc::channel();
        let (release_response_tx, release_response_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let read = stream.read(&mut request).unwrap();
            assert!(read > 0);
            request_started_tx.send(()).unwrap();
            release_response_rx.recv().unwrap();
            let body = br#"{"service":"yap-server","status":"ok","apiVersion":"1","auth":"not_configured","capabilities":{"batchJobs":true,"liveStreaming":true,"jobStatus":true}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let connector = Arc::new(ServerConnector::default());
        {
            let mut inner = connector.inner.lock().unwrap();
            inner.apply_server_settings(0, true, Some(base_url.clone()));
            assert!(inner.begin_health_request(0, 10));
        }
        let request_connector = Arc::clone(&connector);
        let request = std::thread::spawn(move || {
            tauri::async_runtime::block_on(client::check_health(
                &request_connector.client,
                &base_url,
                false,
            ))
        });

        request_started_rx.recv().unwrap();
        assert_eq!(connector.invalidate(), 1);
        release_response_tx.send(()).unwrap();
        let result = request.join().unwrap();
        server.join().unwrap();

        let mut inner = connector.inner.lock().unwrap();
        assert!(inner
            .finish_health_request(0, result, 20, |_| Duration::ZERO)
            .is_none());
        assert_eq!(
            inner.snapshot().state,
            runtime::state::ServerConnectorState::NotSet
        );
        assert_eq!(inner.snapshot().capabilities, ServerCapabilities::default());
    }

    #[test]
    fn settings_changes_advance_the_connector_generation() {
        let connector = ServerConnector::default();

        assert_eq!(connector.current(), 0);
        assert_eq!(connector.invalidate(), 1);
        assert_eq!(connector.current(), 1);
    }

    #[test]
    fn post_publication_durability_failure_invalidates_generation_and_reports_visible_change() {
        let dir = std::env::temp_dir().join(format!(
            "yap-server-settings-post-publish-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server-settings.json");
        let settings = config::ServerSettings {
            schema_version: config::CURRENT_SCHEMA_VERSION,
            enabled: true,
            base_url: Some("https://visible.example".into()),
        };
        let save_result = config::save_to_path_with_hooks(
            &settings,
            &path,
            false,
            || Ok(()),
            || Ok(()),
            |_, _| Ok(()),
            |_| Err(std::io::Error::other("injected parent fsync failure")),
        );
        let connector = ServerConnector::default();

        let error = finish_settings_save(&connector, save_result).unwrap_err();

        assert_eq!(connector.current(), 1);
        assert!(error.starts_with("Server settings changed, but durability confirmation failed:"));
        assert_eq!(config::load_from_path(&path, false).unwrap(), settings);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn pre_publication_failure_still_leaves_stale_leases_revoked() {
        let connector = ServerConnector::default();
        let result = Err(config::ConfigError::SaveIo(std::io::Error::other(
            "injected staging failure",
        )));

        let error = finish_settings_save(&connector, result).unwrap_err();

        assert_eq!(connector.current(), 1);
        assert!(error.starts_with("Could not save server settings:"));
    }

    #[test]
    fn visible_and_indeterminate_publication_failures_each_invalidate_generation_exactly_once() {
        let cases = [
            config::ConfigError::PublicationFailedAfterVisibleChange {
                source: std::io::Error::from_raw_os_error(1176),
                recovery_path: Some(std::path::PathBuf::from("visible-recovery.json")),
            },
            config::ConfigError::PublicationStateIndeterminate {
                source: std::io::Error::from_raw_os_error(1177),
                recovery_path: Some(std::path::PathBuf::from("indeterminate-recovery.json")),
            },
        ];

        for error in cases {
            let connector = ServerConnector::default();
            let result = finish_settings_save(&connector, Err(error));

            assert!(result.is_err());
            assert_eq!(connector.current(), 1);
        }
    }
}
