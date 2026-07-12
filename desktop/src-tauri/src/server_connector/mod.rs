mod client;
pub mod config;
mod state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
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
        runtime_state: &runtime::RuntimeOrchestratorState,
        app: &tauri::AppHandle,
    ) -> Result<ServerConnectionSnapshot, config::ConfigError> {
        self.with_loaded_settings(config::load, |inner, settings| {
            self.synchronize_settings_locked(inner, &settings, runtime_state, app)
        })
    }

    fn synchronize_settings_locked(
        &self,
        inner: &mut ConnectorInner,
        settings: &config::ServerSettings,
        runtime_state: &runtime::RuntimeOrchestratorState,
        app: &tauri::AppHandle,
    ) -> ServerConnectionSnapshot {
        let mut generation = self.generation.load(Ordering::Acquire);
        if !inner.configuration_matches(generation, settings.enabled, settings.base_url.as_deref())
        {
            if inner.generation() == generation && inner.current_configuration_initialized() {
                generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
            }
            inner.apply_server_settings(generation, settings.enabled, settings.base_url.clone());
            project_transition(runtime_state, app, &inner.snapshot());
        }
        inner.snapshot()
    }

    fn snapshot(&self) -> ServerConnectionSnapshot {
        self.inner
            .lock()
            .expect("server connector poisoned")
            .snapshot()
    }

    async fn refresh(
        &self,
        app: &tauri::AppHandle,
        runtime_state: &runtime::RuntimeOrchestratorState,
    ) -> ServerConnectionSnapshot {
        let generation = self.generation.load(Ordering::Acquire);
        let base_url = {
            let mut inner = self.inner.lock().expect("server connector poisoned");
            let base_url = inner.configured_base_url(generation);
            if base_url.is_none() || !inner.begin_health_request(generation, now_ms()) {
                return inner.snapshot();
            }
            project_transition(runtime_state, app, &inner.snapshot());
            base_url.expect("enabled connector has a base URL")
        };

        let result =
            client::check_health(&self.client, &base_url, allow_insecure_private_server()).await;
        self.accept_health_result(app, runtime_state, generation, result)
    }

    fn accept_health_result(
        &self,
        app: &tauri::AppHandle,
        runtime_state: &runtime::RuntimeOrchestratorState,
        generation: u64,
        result: client::HealthCheckResult,
    ) -> ServerConnectionSnapshot {
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
            project_transition(runtime_state, app, &inner.snapshot());

            if let Some(delay) = transition.retry_after {
                let retry_at_ms = now_ms().saturating_add(duration_ms(delay));
                if inner.arm_retry(generation, retry_at_ms) {
                    let snapshot = inner.snapshot();
                    project_transition(runtime_state, app, &snapshot);
                    let retry_token = inner.retry_token();
                    let task = spawn_retry(app.clone(), generation, retry_token, delay);
                    inner.install_retry_task(task);
                }
            }
        }

        self.snapshot()
    }
}

fn project_transition(
    runtime_state: &runtime::RuntimeOrchestratorState,
    app: &tauri::AppHandle,
    snapshot: &ServerConnectionSnapshot,
) {
    runtime_state.with(|orchestrator| {
        orchestrator.set_server(snapshot.state, snapshot.capabilities);
    });
    if let Err(error) = app.emit("server-connection", snapshot.clone()) {
        crate::stt::log_yap(&format!("server connection event failed: {error}"));
    }
}

fn spawn_retry(
    app: tauri::AppHandle,
    generation: u64,
    retry_token: u64,
    delay: Duration,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        Box::pin(run_scheduled_retry(app, generation, retry_token)).await;
    })
}

async fn run_scheduled_retry(app: tauri::AppHandle, generation: u64, retry_token: u64) {
    let connector = app.state::<ServerConnector>();
    let runtime_state = app.state::<runtime::RuntimeOrchestratorState>();
    let base_url = {
        let mut inner = connector.inner.lock().expect("server connector poisoned");
        if connector.generation.load(Ordering::Acquire) != generation
            || !inner.begin_scheduled_retry(generation, retry_token, now_ms())
        {
            return;
        }
        let Some(base_url) = inner.configured_base_url(generation) else {
            return;
        };
        project_transition(&runtime_state, &app, &inner.snapshot());
        base_url
    };

    let result = client::check_health(
        &connector.client,
        &base_url,
        allow_insecure_private_server(),
    )
    .await;
    connector.accept_health_result(&app, &runtime_state, generation, result);
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
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<ServerConnectionSnapshot, String> {
    crate::authorization::ensure_main(&window)?;
    connector
        .synchronize_from_disk(&runtime_state, &app)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn refresh_server_connection(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
) -> Result<ServerConnectionSnapshot, String> {
    crate::authorization::ensure_main(&window)?;
    connector
        .synchronize_from_disk(&runtime_state, &app)
        .map_err(|error| error.to_string())?;
    Ok(connector.refresh(&app, &runtime_state).await)
}

#[tauri::command]
pub(crate) fn server_settings(
    window: tauri::WebviewWindow,
) -> Result<config::ServerSettings, String> {
    crate::authorization::ensure_main(&window)?;
    config::load().map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn set_server_settings(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
    runtime_state: tauri::State<'_, runtime::RuntimeOrchestratorState>,
    settings: config::ServerSettings,
) -> Result<config::ServerSettings, String> {
    crate::authorization::ensure_main(&window)?;
    let mut inner = connector.inner.lock().expect("server connector poisoned");
    let generation_before = connector.generation.load(Ordering::Acquire);
    let result = finish_settings_save_locked(&connector, &mut inner, config::save(&settings));
    if connector.generation.load(Ordering::Acquire) != generation_before {
        let current = result
            .as_ref()
            .ok()
            .cloned()
            .or_else(|| config::load().ok());
        if let Some(current) = current {
            let generation = connector.generation.load(Ordering::Acquire);
            inner.apply_server_settings(generation, current.enabled, current.base_url.clone());
        }
        project_transition(&runtime_state, &app, &inner.snapshot());
    }
    result
}

#[cfg(test)]
fn finish_settings_save(
    connector: &ServerConnector,
    result: Result<config::ServerSettings, config::ConfigError>,
) -> Result<config::ServerSettings, String> {
    let mut inner = connector.inner.lock().expect("server connector poisoned");
    finish_settings_save_locked(connector, &mut inner, result)
}

fn finish_settings_save_locked(
    connector: &ServerConnector,
    inner: &mut ConnectorInner,
    result: Result<config::ServerSettings, config::ConfigError>,
) -> Result<config::ServerSettings, String> {
    match result {
        Ok(saved) => {
            connector.invalidate_locked(inner);
            Ok(saved)
        }
        Err(error) => {
            if error.settings_may_have_changed() {
                connector.invalidate_locked(inner);
            }
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn pre_publication_failure_does_not_invalidate_generation() {
        let connector = ServerConnector::default();
        let result = Err(config::ConfigError::SaveIo(std::io::Error::other(
            "injected staging failure",
        )));

        let error = finish_settings_save(&connector, result).unwrap_err();

        assert_eq!(connector.current(), 0);
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
