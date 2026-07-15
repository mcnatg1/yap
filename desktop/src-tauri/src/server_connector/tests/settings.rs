use std::time::{SystemTime, UNIX_EPOCH};

use crate::server_connector::{
    client, config,
    desktop::{
        check_health_for_approved_origin, finish_settings_save, requires_server_origin_confirmation,
    },
    ServerConnector,
};

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
fn post_publication_durability_failure_invalidates_generation_and_reports_visible_change() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "yap-server-settings-post-publish-{}-{nonce}",
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
