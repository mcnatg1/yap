use std::time::Duration;

use crate::runtime::state::ServerConnectorState;

use super::client::HealthCheckResult;

const RETRY_SECONDS: [u64; 6] = [1, 2, 4, 8, 15, 30];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerCapabilities {
    pub batch_jobs: bool,
    pub live_streaming: bool,
    pub job_status: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerConnectionSnapshot {
    pub state: ServerConnectorState,
    pub checked_at_ms: Option<u64>,
    pub retry_at_ms: Option<u64>,
    pub api_version: Option<String>,
    pub capabilities: ServerCapabilities,
    pub error_code: Option<String>,
}

impl Default for ServerConnectionSnapshot {
    fn default() -> Self {
        Self {
            state: ServerConnectorState::NotSet,
            checked_at_ms: None,
            retry_at_ms: None,
            api_version: None,
            capabilities: ServerCapabilities::default(),
            error_code: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsDisposition {
    NotSet,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectorTransition {
    pub(crate) retry_after: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
struct InFlightRequest {
    generation: u64,
    started_ready: bool,
}

#[derive(Debug)]
pub(crate) struct ConnectorInner {
    generation: u64,
    initialized: bool,
    settings: SettingsDisposition,
    base_url: Option<String>,
    snapshot: ServerConnectionSnapshot,
    in_flight: Option<InFlightRequest>,
    retry_pending: bool,
    retry_allowed: bool,
    retry_attempt: usize,
    retry_token: u64,
    retry_task: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl Default for ConnectorInner {
    fn default() -> Self {
        Self {
            generation: 0,
            initialized: false,
            settings: SettingsDisposition::NotSet,
            base_url: None,
            snapshot: ServerConnectionSnapshot::default(),
            in_flight: None,
            retry_pending: false,
            retry_allowed: false,
            retry_attempt: 0,
            retry_token: 0,
            retry_task: None,
        }
    }
}

impl ConnectorInner {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn current_configuration_initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn apply_settings(&mut self, generation: u64, settings: SettingsDisposition) {
        self.cancel_retry_task();
        self.generation = generation;
        self.initialized = true;
        self.settings = settings;
        self.base_url = None;
        self.snapshot = ServerConnectionSnapshot {
            state: match settings {
                SettingsDisposition::NotSet | SettingsDisposition::Enabled => {
                    ServerConnectorState::NotSet
                }
                SettingsDisposition::Disabled => ServerConnectorState::Disabled,
            },
            ..ServerConnectionSnapshot::default()
        };
        self.in_flight = None;
        self.retry_pending = false;
        self.retry_allowed = false;
        self.retry_attempt = 0;
    }

    pub(crate) fn apply_server_settings(
        &mut self,
        generation: u64,
        enabled: bool,
        base_url: Option<String>,
    ) {
        let disposition = if !enabled {
            SettingsDisposition::Disabled
        } else if base_url.is_none() {
            SettingsDisposition::NotSet
        } else {
            SettingsDisposition::Enabled
        };
        self.apply_settings(generation, disposition);
        self.base_url = base_url;
    }

    pub(crate) fn configuration_matches(
        &self,
        generation: u64,
        enabled: bool,
        base_url: Option<&str>,
    ) -> bool {
        self.initialized
            && self.generation == generation
            && self.settings
                == if !enabled {
                    SettingsDisposition::Disabled
                } else if base_url.is_none() {
                    SettingsDisposition::NotSet
                } else {
                    SettingsDisposition::Enabled
                }
            && self.base_url.as_deref() == base_url
    }

    pub(crate) fn configured_base_url(&self, generation: u64) -> Option<String> {
        (self.generation == generation && self.settings == SettingsDisposition::Enabled)
            .then(|| self.base_url.clone())
            .flatten()
    }

    pub(crate) fn snapshot(&self) -> ServerConnectionSnapshot {
        self.snapshot.clone()
    }

    pub(crate) fn begin_health_request(&mut self, generation: u64, now_ms: u64) -> bool {
        if generation != self.generation
            || self.settings != SettingsDisposition::Enabled
            || self.in_flight.is_some()
        {
            return false;
        }

        let started_ready = self.snapshot.state == ServerConnectorState::Ready;
        self.cancel_retry_task();
        self.in_flight = Some(InFlightRequest {
            generation,
            started_ready,
        });
        self.retry_pending = false;
        self.retry_allowed = false;
        self.snapshot.retry_at_ms = None;
        self.snapshot.error_code = None;
        if !started_ready {
            self.snapshot.state = ServerConnectorState::Connecting;
            self.snapshot.checked_at_ms = Some(now_ms);
            self.snapshot.api_version = None;
            self.snapshot.capabilities = ServerCapabilities::default();
        }
        true
    }

    pub(crate) fn finish_health_request<Jitter>(
        &mut self,
        generation: u64,
        result: HealthCheckResult,
        now_ms: u64,
        jitter: Jitter,
    ) -> Option<ConnectorTransition>
    where
        Jitter: FnOnce(Duration) -> Duration,
    {
        let request = self.in_flight?;
        if generation != self.generation || request.generation != generation {
            return None;
        }
        self.in_flight = None;
        self.snapshot.checked_at_ms = Some(now_ms);
        self.snapshot.retry_at_ms = None;
        self.snapshot.capabilities = ServerCapabilities::default();
        self.snapshot.error_code = None;

        let retry_after = match result {
            HealthCheckResult::Ready {
                api_version,
                capabilities,
            } => {
                self.snapshot.state = ServerConnectorState::Ready;
                self.snapshot.api_version = Some(api_version);
                self.snapshot.capabilities = capabilities;
                self.retry_attempt = 0;
                self.retry_allowed = false;
                None
            }
            HealthCheckResult::SignInRequired { api_version } => {
                self.snapshot.state = ServerConnectorState::SignInRequired;
                self.snapshot.api_version = api_version;
                self.retry_attempt = 0;
                self.retry_allowed = false;
                None
            }
            HealthCheckResult::Offline {
                api_version,
                error_code,
                retryable,
            } => {
                self.snapshot.state = if retryable && request.started_ready {
                    ServerConnectorState::Retrying
                } else {
                    ServerConnectorState::Offline
                };
                self.snapshot.api_version = api_version;
                self.snapshot.error_code = Some(error_code.to_owned());
                self.retry_allowed = retryable;
                retryable.then(|| {
                    let delay = retry_delay(self.retry_attempt, jitter);
                    self.retry_attempt = self.retry_attempt.saturating_add(1);
                    delay
                })
            }
        };

        Some(ConnectorTransition { retry_after })
    }

    pub(crate) fn arm_retry(&mut self, generation: u64, retry_at_ms: u64) -> bool {
        if generation != self.generation
            || self.settings != SettingsDisposition::Enabled
            || !self.retry_allowed
            || self.retry_pending
            || self.in_flight.is_some()
        {
            return false;
        }
        self.retry_pending = true;
        self.retry_token = self.retry_token.wrapping_add(1);
        self.snapshot.state = ServerConnectorState::Retrying;
        self.snapshot.retry_at_ms = Some(retry_at_ms);
        true
    }

    pub(crate) fn retry_token(&self) -> u64 {
        self.retry_token
    }

    pub(crate) fn install_retry_task(&mut self, task: tauri::async_runtime::JoinHandle<()>) {
        self.cancel_retry_task();
        self.retry_task = Some(task);
    }

    fn cancel_retry_task(&mut self) {
        if let Some(task) = self.retry_task.take() {
            task.abort();
        }
    }

    pub(crate) fn begin_scheduled_retry(&mut self, generation: u64, retry_token: u64) -> bool {
        if generation != self.generation
            || retry_token != self.retry_token
            || self.settings != SettingsDisposition::Enabled
            || !self.retry_pending
            || self.in_flight.is_some()
        {
            return false;
        }
        self.retry_task.take();
        self.retry_pending = false;
        self.retry_allowed = false;
        self.snapshot.retry_at_ms = None;
        self.snapshot.state = ServerConnectorState::Connecting;
        self.in_flight = Some(InFlightRequest {
            generation,
            started_ready: false,
        });
        true
    }
}

pub(crate) fn retry_delay<Jitter>(attempt: usize, jitter: Jitter) -> Duration
where
    Jitter: FnOnce(Duration) -> Duration,
{
    let base = Duration::from_secs(RETRY_SECONDS[attempt.min(RETRY_SECONDS.len() - 1)]);
    let maximum_jitter = base / 5;
    base.saturating_add(jitter(base).min(maximum_jitter))
}

pub(crate) fn production_jitter(base: Duration) -> Duration {
    use std::hash::{Hash, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::process::id().hash(&mut hasher);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    SEQUENCE.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
    let maximum_nanos = (base.as_nanos() / 5) as u64;
    Duration::from_nanos(hasher.finish() % maximum_nanos.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{retry_delay, ConnectorInner, ServerCapabilities, SettingsDisposition};
    use crate::runtime::state::ServerConnectorState;
    use crate::server_connector::client::HealthCheckResult;

    fn zero_jitter(_: Duration) -> Duration {
        Duration::ZERO
    }

    fn enabled(inner: &mut ConnectorInner, generation: u64) {
        inner.apply_settings(generation, SettingsDisposition::Enabled);
    }

    #[test]
    fn only_the_newest_generation_can_complete_a_request() {
        let mut inner = ConnectorInner::default();
        enabled(&mut inner, 1);
        assert!(inner.begin_health_request(1, 10));
        inner.apply_settings(2, SettingsDisposition::Disabled);

        let transition = inner.finish_health_request(
            1,
            HealthCheckResult::Ready {
                api_version: "1".to_owned(),
                capabilities: ServerCapabilities {
                    batch_jobs: true,
                    live_streaming: true,
                    job_status: true,
                },
            },
            20,
            zero_jitter,
        );

        assert!(transition.is_none());
        assert_eq!(inner.snapshot().state, ServerConnectorState::Disabled);
        assert_eq!(inner.snapshot().capabilities, ServerCapabilities::default());
    }

    #[test]
    fn one_generation_owns_at_most_one_health_request() {
        let mut inner = ConnectorInner::default();
        enabled(&mut inner, 7);

        assert!(inner.begin_health_request(7, 10));
        assert!(!inner.begin_health_request(7, 11));
        assert!(inner
            .finish_health_request(
                7,
                HealthCheckResult::Ready {
                    api_version: "1".to_owned(),
                    capabilities: ServerCapabilities::default(),
                },
                12,
                zero_jitter,
            )
            .is_some());
        assert!(inner.begin_health_request(7, 13));
    }

    #[test]
    fn disabled_and_not_set_never_request_or_retry() {
        for disposition in [SettingsDisposition::NotSet, SettingsDisposition::Disabled] {
            let mut inner = ConnectorInner::default();
            inner.apply_settings(3, disposition);

            assert!(!inner.begin_health_request(3, 10));
            assert!(!inner.arm_retry(3, 11));
            assert_eq!(inner.snapshot().retry_at_ms, None);
        }
    }

    #[test]
    fn retry_delay_is_bounded_and_deterministic_with_zero_jitter() {
        let expected = [1, 2, 4, 8, 15, 30, 30, 30];
        let actual = (0..expected.len())
            .map(|attempt| retry_delay(attempt, zero_jitter).as_secs())
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn initial_failure_projects_offline_then_one_retry_timer_and_connecting() {
        let mut inner = ConnectorInner::default();
        enabled(&mut inner, 1);
        assert!(inner.begin_health_request(1, 100));
        assert_eq!(inner.snapshot().state, ServerConnectorState::Connecting);

        let transition = inner
            .finish_health_request(
                1,
                HealthCheckResult::Offline {
                    api_version: None,
                    error_code: "CONNECTION_FAILED",
                    retryable: true,
                },
                200,
                zero_jitter,
            )
            .unwrap();

        assert_eq!(inner.snapshot().state, ServerConnectorState::Offline);
        assert_eq!(transition.retry_after, Some(Duration::from_secs(1)));
        assert!(inner.arm_retry(1, 1_200));
        assert!(!inner.arm_retry(1, 1_200));
        assert_eq!(inner.snapshot().state, ServerConnectorState::Retrying);
        assert_eq!(inner.snapshot().retry_at_ms, Some(1_200));
        let retry_token = inner.retry_token();
        assert!(inner.begin_scheduled_retry(1, retry_token));
        assert_eq!(inner.snapshot().state, ServerConnectorState::Connecting);
    }

    #[test]
    fn physically_fired_retry_ignores_wall_clock_rollback() {
        let mut inner = ConnectorInner::default();
        enabled(&mut inner, 2);
        assert!(inner.begin_health_request(2, 100));
        let transition = inner
            .finish_health_request(
                2,
                HealthCheckResult::Offline {
                    api_version: None,
                    error_code: "CONNECTION_FAILED",
                    retryable: true,
                },
                200,
                zero_jitter,
            )
            .unwrap();
        assert_eq!(transition.retry_after, Some(Duration::from_secs(1)));
        assert!(inner.arm_retry(2, 1_200));

        let retry_token = inner.retry_token();
        assert!(inner.begin_scheduled_retry(2, retry_token));
        assert_eq!(inner.snapshot().state, ServerConnectorState::Connecting);
        assert_eq!(inner.snapshot().retry_at_ms, None);
    }

    #[test]
    fn failed_explicit_refresh_from_ready_enters_retrying() {
        let mut inner = ConnectorInner::default();
        enabled(&mut inner, 1);
        assert!(inner.begin_health_request(1, 10));
        inner
            .finish_health_request(
                1,
                HealthCheckResult::Ready {
                    api_version: "1".to_owned(),
                    capabilities: ServerCapabilities::default(),
                },
                20,
                zero_jitter,
            )
            .unwrap();
        assert!(inner.begin_health_request(1, 30));

        let transition = inner
            .finish_health_request(
                1,
                HealthCheckResult::Offline {
                    api_version: None,
                    error_code: "SERVER_ERROR",
                    retryable: true,
                },
                40,
                zero_jitter,
            )
            .unwrap();

        assert_eq!(inner.snapshot().state, ServerConnectorState::Retrying);
        assert_eq!(transition.retry_after, Some(Duration::from_secs(1)));
        assert_eq!(inner.snapshot().capabilities, ServerCapabilities::default());
    }

    #[test]
    fn incompatible_response_clears_capabilities_and_never_retries() {
        let mut inner = ConnectorInner::default();
        enabled(&mut inner, 4);
        assert!(inner.begin_health_request(4, 10));

        let transition = inner
            .finish_health_request(
                4,
                HealthCheckResult::Offline {
                    api_version: Some("2".to_owned()),
                    error_code: "INCOMPATIBLE_API_VERSION",
                    retryable: false,
                },
                20,
                zero_jitter,
            )
            .unwrap();

        assert_eq!(transition.retry_after, None);
        assert_eq!(inner.snapshot().state, ServerConnectorState::Offline);
        assert_eq!(inner.snapshot().capabilities, ServerCapabilities::default());
        assert_eq!(
            inner.snapshot().error_code.as_deref(),
            Some("INCOMPATIBLE_API_VERSION")
        );
        assert!(!inner.arm_retry(4, 1_020));
    }

    #[test]
    fn sign_in_required_never_retries() {
        let mut inner = ConnectorInner::default();
        enabled(&mut inner, 5);
        assert!(inner.begin_health_request(5, 10));

        let transition = inner
            .finish_health_request(
                5,
                HealthCheckResult::SignInRequired {
                    api_version: Some("1".to_owned()),
                },
                20,
                zero_jitter,
            )
            .unwrap();

        assert_eq!(transition.retry_after, None);
        assert_eq!(inner.snapshot().state, ServerConnectorState::SignInRequired);
        assert!(!inner.arm_retry(5, 1_020));
    }

    #[test]
    fn explicit_refresh_aborts_the_existing_retry_timer() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        tauri::async_runtime::block_on(async {
            let mut inner = ConnectorInner::default();
            enabled(&mut inner, 8);
            assert!(inner.begin_health_request(8, 10));
            let transition = inner
                .finish_health_request(
                    8,
                    HealthCheckResult::Offline {
                        api_version: None,
                        error_code: "CONNECTION_FAILED",
                        retryable: true,
                    },
                    20,
                    zero_jitter,
                )
                .unwrap();
            assert_eq!(transition.retry_after, Some(Duration::from_secs(1)));
            assert!(inner.arm_retry(8, 1_020));

            let fired = Arc::new(AtomicBool::new(false));
            let fired_by_timer = Arc::clone(&fired);
            let timer = tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                fired_by_timer.store(true, Ordering::Release);
            });
            inner.install_retry_task(timer);

            assert!(inner.begin_health_request(8, 30));
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(!fired.load(Ordering::Acquire));
        });
    }
}
