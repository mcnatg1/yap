use std::time::Duration;

use reqwest::{Client, StatusCode};

use super::config;
use super::state::ServerCapabilities;

const MAX_HEALTH_BYTES: usize = 64 * 1024;
const SUPPORTED_API_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HealthCheckResult {
    Ready {
        api_version: String,
        capabilities: ServerCapabilities,
    },
    SignInRequired {
        api_version: Option<String>,
    },
    Offline {
        api_version: Option<String>,
        error_code: &'static str,
        retryable: bool,
    },
}

pub(crate) fn bounded_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        // reqwest has no cookie store unless its optional cookies feature is enabled.
        .build()
}

pub(crate) async fn check_health(
    client: &Client,
    base_url: &str,
    allow_insecure_private: bool,
) -> HealthCheckResult {
    let normalized = match config::validate_base_url(base_url, allow_insecure_private) {
        Ok(normalized) => normalized,
        Err(_) => return offline(None, "INVALID_SERVER_URL", false),
    };
    let mut health_url = match reqwest::Url::parse(&normalized) {
        Ok(url) => url,
        Err(_) => return offline(None, "INVALID_SERVER_URL", false),
    };
    health_url.set_path("/v1/health");

    let response = match client
        .get(health_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_connect() => return offline(None, "CONNECTION_FAILED", true),
        Err(error) if error.is_timeout() => return offline(None, "REQUEST_TIMEOUT", true),
        Err(_) => return offline(None, "CONNECTION_FAILED", true),
    };

    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return HealthCheckResult::SignInRequired { api_version: None };
        }
        status if status.is_server_error() => return offline(None, "SERVER_ERROR", true),
        StatusCode::OK => {}
        _ => return offline(None, "UNEXPECTED_HTTP_STATUS", true),
    }

    let body = match read_bounded(response).await {
        Ok(body) => body,
        Err(ReadHealthBodyError::TooLarge) => {
            return offline(None, "HEALTH_RESPONSE_TOO_LARGE", true);
        }
        Err(ReadHealthBodyError::Transport(error)) if error.is_timeout() => {
            return offline(None, "REQUEST_TIMEOUT", true);
        }
        Err(ReadHealthBodyError::Transport(_)) => {
            return offline(None, "CONNECTION_FAILED", true);
        }
    };

    project_health(&body)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HealthEnvelope {
    service: String,
    status: String,
    api_version: String,
    auth: String,
    capabilities: Option<serde_json::Value>,
}

fn project_health(body: &[u8]) -> HealthCheckResult {
    let envelope: HealthEnvelope = match serde_json::from_slice(body) {
        Ok(envelope) => envelope,
        Err(_) => return offline(None, "MALFORMED_HEALTH_RESPONSE", true),
    };
    let api_version = Some(envelope.api_version.clone());
    if envelope.api_version != SUPPORTED_API_VERSION {
        return offline(api_version, "INCOMPATIBLE_API_VERSION", false);
    }
    let Some(capability_value) = envelope.capabilities else {
        return offline(api_version, "INCOMPATIBLE_CAPABILITIES", false);
    };
    let capabilities: ServerCapabilities = match serde_json::from_value(capability_value) {
        Ok(capabilities) => capabilities,
        Err(_) => return offline(api_version, "INCOMPATIBLE_CAPABILITIES", false),
    };
    if envelope.service != "yap-server" || envelope.status != "ok" {
        return offline(api_version, "MALFORMED_HEALTH_RESPONSE", true);
    }
    match envelope.auth.as_str() {
        "not_configured" => HealthCheckResult::Ready {
            api_version: envelope.api_version,
            capabilities,
        },
        "required" => HealthCheckResult::SignInRequired { api_version },
        _ => offline(api_version, "MALFORMED_HEALTH_RESPONSE", true),
    }
}

fn offline(
    api_version: Option<String>,
    error_code: &'static str,
    retryable: bool,
) -> HealthCheckResult {
    HealthCheckResult::Offline {
        api_version,
        error_code,
        retryable,
    }
}

#[derive(Debug)]
enum ReadHealthBodyError {
    TooLarge,
    Transport(reqwest::Error),
}

async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, ReadHealthBodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HEALTH_BYTES as u64)
    {
        return Err(ReadHealthBodyError::TooLarge);
    }
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(MAX_HEALTH_BYTES as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(ReadHealthBodyError::Transport)?
    {
        if body.len().saturating_add(chunk.len()) > MAX_HEALTH_BYTES {
            return Err(ReadHealthBodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use super::{bounded_client, check_health, HealthCheckResult};
    use crate::server_connector::state::ServerCapabilities;

    struct Fixture {
        address: SocketAddr,
        worker: Option<JoinHandle<()>>,
    }

    impl Fixture {
        fn response(status: &str, body: impl Into<Vec<u8>>, delay: Duration) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let status = status.to_owned();
            let body = body.into();
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&body);
            });
            Self {
                address,
                worker: Some(worker),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.address)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    fn check(base_url: &str) -> HealthCheckResult {
        let client = bounded_client().unwrap();
        tauri::async_runtime::block_on(check_health(&client, base_url, false))
    }

    fn healthy_body(api_version: &str, auth: &str, capabilities: &str) -> String {
        format!(
            r#"{{"service":"yap-server","status":"ok","apiVersion":"{api_version}","auth":"{auth}","capabilities":{capabilities}}}"#
        )
    }

    #[test]
    fn healthy_v1_response_advertises_only_server_capabilities() {
        let fixture = Fixture::response(
            "200 OK",
            healthy_body(
                "1",
                "not_configured",
                r#"{"batchJobs":true,"liveStreaming":false,"jobStatus":true}"#,
            ),
            Duration::ZERO,
        );

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Ready {
                api_version: "1".to_owned(),
                capabilities: ServerCapabilities {
                    batch_jobs: true,
                    live_streaming: false,
                    job_status: true,
                },
            }
        );
    }

    #[test]
    fn unsupported_version_fails_closed_without_retry() {
        let fixture = Fixture::response(
            "200 OK",
            healthy_body(
                "2",
                "not_configured",
                r#"{"batchJobs":true,"liveStreaming":true,"jobStatus":true}"#,
            ),
            Duration::ZERO,
        );

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: Some("2".to_owned()),
                error_code: "INCOMPATIBLE_API_VERSION",
                retryable: false,
            }
        );
    }

    #[test]
    fn malformed_capabilities_fail_closed_as_incompatible() {
        let fixture = Fixture::response(
            "200 OK",
            healthy_body(
                "1",
                "not_configured",
                r#"{"batchJobs":"yes","liveStreaming":true,"jobStatus":true}"#,
            ),
            Duration::ZERO,
        );

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: Some("1".to_owned()),
                error_code: "INCOMPATIBLE_CAPABILITIES",
                retryable: false,
            }
        );
    }

    #[test]
    fn absent_capability_object_fails_closed_without_retry() {
        let fixture = Fixture::response(
            "200 OK",
            br#"{"service":"yap-server","status":"ok","apiVersion":"1","auth":"not_configured"}"#
                .to_vec(),
            Duration::ZERO,
        );

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: Some("1".to_owned()),
                error_code: "INCOMPATIBLE_CAPABILITIES",
                retryable: false,
            }
        );
    }

    #[test]
    fn missing_capability_field_fails_closed_without_retry() {
        let fixture = Fixture::response(
            "200 OK",
            healthy_body(
                "1",
                "not_configured",
                r#"{"batchJobs":true,"liveStreaming":true}"#,
            ),
            Duration::ZERO,
        );

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: Some("1".to_owned()),
                error_code: "INCOMPATIBLE_CAPABILITIES",
                retryable: false,
            }
        );
    }

    #[test]
    fn malformed_json_is_retryable_and_fail_closed() {
        let fixture = Fixture::response("200 OK", b"{not-json".to_vec(), Duration::ZERO);

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: None,
                error_code: "MALFORMED_HEALTH_RESPONSE",
                retryable: true,
            }
        );
    }

    #[test]
    fn authentication_status_and_health_auth_require_sign_in() {
        for status in ["401 Unauthorized", "403 Forbidden"] {
            let fixture = Fixture::response(status, Vec::new(), Duration::ZERO);
            assert_eq!(
                check(&fixture.base_url()),
                HealthCheckResult::SignInRequired { api_version: None }
            );
        }

        let fixture = Fixture::response(
            "200 OK",
            healthy_body(
                "1",
                "required",
                r#"{"batchJobs":true,"liveStreaming":true,"jobStatus":true}"#,
            ),
            Duration::ZERO,
        );
        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::SignInRequired {
                api_version: Some("1".to_owned()),
            }
        );
    }

    #[test]
    fn server_errors_and_connection_refusal_are_retryable() {
        let fixture = Fixture::response("500 Internal Server Error", Vec::new(), Duration::ZERO);
        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: None,
                error_code: "SERVER_ERROR",
                retryable: true,
            }
        );

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let refused = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        assert_eq!(
            check(&refused),
            HealthCheckResult::Offline {
                api_version: None,
                error_code: "CONNECTION_FAILED",
                retryable: true,
            }
        );
    }

    #[test]
    fn delayed_response_hits_the_three_second_total_timeout() {
        let fixture = Fixture::response(
            "200 OK",
            healthy_body(
                "1",
                "not_configured",
                r#"{"batchJobs":false,"liveStreaming":false,"jobStatus":false}"#,
            ),
            Duration::from_millis(3_100),
        );

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: None,
                error_code: "REQUEST_TIMEOUT",
                retryable: true,
            }
        );
    }

    #[test]
    fn response_body_is_bounded_to_sixty_four_kibibytes() {
        let fixture = Fixture::response("200 OK", vec![b'x'; 65_537], Duration::ZERO);

        assert_eq!(
            check(&fixture.base_url()),
            HealthCheckResult::Offline {
                api_version: None,
                error_code: "HEALTH_RESPONSE_TOO_LARGE",
                retryable: true,
            }
        );
    }

    #[test]
    fn invalid_url_is_rejected_before_network_io() {
        assert_eq!(
            check("http://example.com"),
            HealthCheckResult::Offline {
                api_version: None,
                error_code: "INVALID_SERVER_URL",
                retryable: false,
            }
        );
    }
}
