use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use super::{
    allow_insecure_private_server, client, config, ServerConnectionSnapshot, ServerConnector,
};

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
