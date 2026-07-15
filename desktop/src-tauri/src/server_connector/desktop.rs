use std::time::Duration;

use tauri::{Emitter, Manager};

use super::{
    allow_insecure_private_server, client, config, ServerConnectionSnapshot, ServerConnector,
};

impl ServerConnector {
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

pub(super) async fn check_health_for_approved_origin<Authorize>(
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

pub(super) fn connection_status(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    connector: tauri::State<'_, ServerConnector>,
) -> Result<ServerConnectionSnapshot, String> {
    crate::authorization::ensure_main(&window)?;
    connector
        .synchronize_from_disk(&app)
        .map_err(|error| error.to_string())
}

pub(super) async fn refresh_connection(
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

pub(super) fn load_settings(
    window: tauri::WebviewWindow,
) -> Result<config::ServerSettings, String> {
    crate::authorization::ensure_main(&window)?;
    config::load().map_err(|error| error.to_string())
}

pub(super) async fn save_settings(
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
    // publication. If approval publication fails, the origin stays unauthorized.
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

pub(super) fn requires_server_origin_confirmation(
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
pub(super) fn finish_settings_save(
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
