use std::time::{Duration, UNIX_EPOCH};

use crate::audio::session::{SessionId, SessionMetadata, SessionMode, SessionOrigin, TriggerMode};

use super::{
    validate_development_batch_base_url, ApiError, BatchClientError, CaptureChunkReference,
    CaptureManifestReference, ContentIdentity, CreateRecordingJobRequest, ServerReplayKey,
    UploadTrack,
};

#[test]
fn unauthenticated_audio_transport_accepts_only_loopback_tunnel_origins() {
    assert_eq!(
        validate_development_batch_base_url("http://127.0.0.1:18765").unwrap(),
        "http://127.0.0.1:18765"
    );
    assert_eq!(
        validate_development_batch_base_url("http://[::1]:18765/v1").unwrap(),
        "http://[::1]:18765"
    );
    assert!(validate_development_batch_base_url("http://localhost:18765").is_err());
    assert!(validate_development_batch_base_url("http://192.168.50.1:18765").is_err());
    assert!(validate_development_batch_base_url("https://yap.internal").is_err());
}

#[test]
fn persisted_create_request_round_trips_strictly_before_resume() {
    let started = UNIX_EPOCH + Duration::from_secs(1_720_000_000);
    let session_id = "s-persisted-request";
    let request = CreateRecordingJobRequest {
        display_name: "interview.wav".into(),
        metadata: SessionMetadata::new(
            SessionId::new(session_id).unwrap(),
            SessionMode::Meeting,
            SessionOrigin::ImportedFile,
            TriggerMode::Toggle,
            started,
            None,
            Some("en-US".into()),
            None,
            vec!["en-US".into()],
            Some(started + Duration::from_secs(3600)),
        )
        .unwrap(),
        tracks: vec![UploadTrack {
            track_id: "track-1".into(),
            source: serde_json::json!({"kind": "imported", "provenance": "unknown"}),
            device_id: None,
            original_sample_rate_hz: 16_000,
            original_channels: 1,
        }],
        route: "server_batch".into(),
        capture_manifest: CaptureManifestReference {
            schema_version: 1,
            session_id: session_id.into(),
            sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
            byte_length: 200,
        },
        chunks: vec![CaptureChunkReference {
            replay_key: ServerReplayKey {
                schema_version: 1,
                session_id: session_id.into(),
                track_id: "track-1".into(),
                sequence_start: 0,
                sequence_end: 159,
            },
            content_identity: ContentIdentity {
                sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into(),
                byte_length: 320,
            },
            audio_codec: "pcm_s16le".into(),
            sample_rate_hz: 16_000,
            channels: 1,
            start_ms: 0,
            duration_ms: 10,
        }],
    };
    let encoded = serde_json::to_string(&request).unwrap();
    let original_key = request.create_idempotency_key().unwrap();

    assert_eq!(
        CreateRecordingJobRequest::decode_persisted(&encoded).unwrap(),
        request
    );
    assert_eq!(request.create_idempotency_key().unwrap(), original_key);
    let mut new_attempt = request.clone();
    new_attempt.display_name = "a distinct immutable request".into();
    assert_ne!(new_attempt.create_idempotency_key().unwrap(), original_key);
    let with_unknown = encoded.replacen('{', r#"{"unexpected":true,"#, 1);
    assert!(CreateRecordingJobRequest::decode_persisted(&with_unknown).is_err());
    let mut missing_retention = request.clone();
    missing_retention.metadata.retention_expires_at_utc = None;
    assert!(CreateRecordingJobRequest::decode_persisted(
        &serde_json::to_string(&missing_retention).unwrap()
    )
    .is_err());
    let mut unbounded_retention = request.clone();
    unbounded_retention.metadata.retention_expires_at_utc = Some("2126-07-14T21:00:00Z".into());
    assert!(CreateRecordingJobRequest::decode_persisted(
        &serde_json::to_string(&unbounded_retention).unwrap()
    )
    .is_err());

    const FOUR_HOURS_PCM_BYTES: u64 = 16_000 * 2 * 4 * 60 * 60;
    let mut oversized = request;
    let chunk_bytes = 960_000_u64;
    let chunk_frames = chunk_bytes / 2;
    let chunk_duration_ms = 30_000_u32;
    oversized.chunks = (0..=(FOUR_HOURS_PCM_BYTES / chunk_bytes))
        .map(|index| {
            let sequence_start = index * chunk_frames;
            CaptureChunkReference {
                replay_key: ServerReplayKey {
                    schema_version: 1,
                    session_id: session_id.into(),
                    track_id: "track-1".into(),
                    sequence_start,
                    sequence_end: sequence_start + chunk_frames - 1,
                },
                content_identity: ContentIdentity {
                    sha256: "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                        .into(),
                    byte_length: chunk_bytes,
                },
                audio_codec: "pcm_s16le".into(),
                sample_rate_hz: 16_000,
                channels: 1,
                start_ms: index * u64::from(chunk_duration_ms),
                duration_ms: chunk_duration_ms,
            }
        })
        .collect();
    assert!(CreateRecordingJobRequest::decode_persisted(
        &serde_json::to_string(&oversized).unwrap()
    )
    .is_err());
}

#[test]
fn server_retryability_is_preserved_as_typed_transport_state() {
    let retryable = BatchClientError::Api {
        status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
        code: "POOL_BUSY".into(),
        retryable: true,
    };
    let terminal = BatchClientError::Api {
        status: reqwest::StatusCode::CONFLICT,
        code: "MANIFEST_CONFLICT".into(),
        retryable: false,
    };

    assert!(retryable.is_retryable());
    assert!(!terminal.is_retryable());
    assert!(!BatchClientError::MalformedResponse.is_retryable());
}

#[test]
fn server_error_fields_are_bounded_before_logging_or_retry_decisions() {
    let valid = ApiError {
        code: "POOL_BUSY".into(),
        message: "Try again.".into(),
        retryable: true,
        request_id: "job-abc123".into(),
    };
    assert!(valid.is_valid());

    let mut injected_line = valid.clone();
    injected_line.message = "Try again.\nforged log entry".into();
    assert!(!injected_line.is_valid());

    let mut invalid_request_id = valid;
    invalid_request_id.request_id = "../../outside".into();
    assert!(!invalid_request_id.is_valid());
}
