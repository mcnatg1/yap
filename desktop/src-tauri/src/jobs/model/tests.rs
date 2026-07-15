use super::view::{PipelineStageStatus, RecordingPipelineState};
use super::*;

#[test]
fn database_enums_round_trip_and_reject_unknown_values() {
    for status in RecordingJobStatus::ALL {
        assert_eq!(RecordingJobStatus::from_db(status.as_db()).unwrap(), status);
    }
    for mode in SessionMode::ALL {
        assert_eq!(SessionMode::from_db(mode.as_db()).unwrap(), mode);
    }
    for origin in SessionOrigin::ALL {
        assert_eq!(SessionOrigin::from_db(origin.as_db()).unwrap(), origin);
    }
    for route in RecordingRoute::ALL {
        assert_eq!(RecordingRoute::from_db(route.as_db()).unwrap(), route);
    }
    assert!(matches!(
        RecordingJobStatus::from_db("invented_ui_state"),
        Err(JobLedgerError::CorruptValue {
            field: "status",
            ..
        })
    ));
    assert!(SessionMode::from_db("podcast").is_err());
    assert!(SessionOrigin::from_db("cloud_upload").is_err());
    assert!(RecordingRoute::from_db("direct").is_err());
}

#[test]
fn transition_policy_is_pure_and_cancellation_is_narrow() {
    assert_eq!(
        transition_policy(
            RecordingJobStatus::Accepted,
            RecordingJobStatus::Preflighting
        ),
        TransitionPolicy::Ordinary
    );
    assert_eq!(
        transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Preflighting),
        TransitionPolicy::Retry
    );
    assert_eq!(
        transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Uploading),
        TransitionPolicy::Forbidden
    );
    assert_eq!(
        transition_policy(RecordingJobStatus::Failed, RecordingJobStatus::Cancelled),
        TransitionPolicy::Dismiss
    );

    for status in [
        RecordingJobStatus::Accepted,
        RecordingJobStatus::BlockedSetupRequired,
        RecordingJobStatus::BlockedServerUnavailable,
        RecordingJobStatus::BlockedSignInRequired,
        RecordingJobStatus::QueuedLocalFallback,
        RecordingJobStatus::QueuedServer,
        RecordingJobStatus::Preprocessing,
        RecordingJobStatus::Uploading,
        RecordingJobStatus::ServerProcessing,
        RecordingJobStatus::Saving,
    ] {
        assert_eq!(
            transition_policy(status, RecordingJobStatus::Cancelled),
            TransitionPolicy::Cancellation,
            "{status:?} should be cancellable"
        );
    }
    for status in [
        RecordingJobStatus::Preflighting,
        RecordingJobStatus::LocalTranscribing,
        RecordingJobStatus::DiarizationQueued,
        RecordingJobStatus::DiarizationRunning,
        RecordingJobStatus::Complete,
        RecordingJobStatus::Partial,
        RecordingJobStatus::Cancelled,
    ] {
        assert_eq!(
            transition_policy(status, RecordingJobStatus::Cancelled),
            TransitionPolicy::Forbidden,
            "{status:?} must not be cancellable"
        );
    }
}

#[test]
fn view_projection_uses_the_camel_case_typescript_contract() {
    let record = fixture_record();
    let value = serde_json::to_value(RecordingJobView::from_record(&record)).unwrap();

    assert_eq!(value["id"], "job-view");
    assert_eq!(
        value["sourcePath"],
        record.source_path.unwrap().display().to_string()
    );
    assert_eq!(value["name"], "Board meeting");
    assert_eq!(value["sessionMode"], "meeting");
    assert_eq!(value["sessionOrigin"], "importedFile");
    assert_eq!(value["status"], "queued_server");
    assert_eq!(value["route"], "serverBatch");
    assert!(value.get("job_id").is_none());
    assert!(value.get("attempt_count").is_none());
}

#[test]
fn every_durable_status_projects_only_proven_pipeline_progress() {
    use PipelineStageStatus::{
        Done as D, NotStarted as N, Queued as Q, Running as R, Skipped as S,
    };
    use RecordingJobStatus as J;
    use RecordingRoute::{LocalFallback as Local, ServerBatch as Server};

    let pipeline = |preprocessing, transcription, alignment, diarization, postprocessing| {
        RecordingPipelineState {
            intake: D,
            preprocessing,
            transcription,
            alignment,
            diarization,
            postprocessing,
        }
    };
    let cases = [
        (J::Accepted, None, pipeline(N, N, N, N, N)),
        (J::Preflighting, None, pipeline(N, N, N, N, N)),
        (J::BlockedSetupRequired, None, pipeline(N, N, N, N, N)),
        (
            J::BlockedServerUnavailable,
            Some(Server),
            pipeline(N, N, N, N, N),
        ),
        (
            J::BlockedSignInRequired,
            Some(Server),
            pipeline(N, N, N, N, N),
        ),
        (J::QueuedLocalFallback, Some(Local), pipeline(S, N, N, N, N)),
        (J::QueuedServer, Some(Server), pipeline(N, N, N, N, N)),
        (J::Preprocessing, Some(Server), pipeline(R, N, N, N, N)),
        (J::Uploading, Some(Server), pipeline(D, N, N, N, N)),
        (J::ServerProcessing, Some(Server), pipeline(D, R, N, N, N)),
        (J::LocalTranscribing, Some(Local), pipeline(S, R, N, N, N)),
        (J::Saving, Some(Server), pipeline(D, D, N, N, R)),
        (J::DiarizationQueued, Some(Server), pipeline(D, D, D, Q, N)),
        (J::DiarizationRunning, Some(Server), pipeline(D, D, D, R, N)),
        (J::Complete, Some(Server), pipeline(D, D, N, N, D)),
        (J::Partial, Some(Server), pipeline(D, D, N, N, N)),
        (J::Failed, Some(Server), pipeline(D, N, N, N, N)),
        (J::Cancelled, Some(Server), pipeline(N, N, N, N, N)),
        (J::Saving, Some(Local), pipeline(S, D, N, N, R)),
        (J::Complete, Some(Local), pipeline(S, D, N, N, D)),
    ];

    for (status, route, expected) in cases {
        let mut record = fixture_record();
        record.status = status;
        record.route = route;

        assert_eq!(
            RecordingJobView::from_record(&record).pipeline,
            expected,
            "unexpected projection for {status:?} with {route:?}"
        );
    }
}

fn fixture_record() -> RecordingJobRecord {
    RecordingJobRecord {
        job_id: "job-view".into(),
        session_mode: SessionMode::Meeting,
        session_origin: SessionOrigin::ImportedFile,
        source_path: Some(std::env::temp_dir().join("meeting.wav")),
        source_ownership: SourceOwnership::External,
        output_path: None,
        display_name: "Board meeting".into(),
        status: RecordingJobStatus::QueuedServer,
        route: Some(RecordingRoute::ServerBatch),
        attempt_count: 0,
        next_attempt_at_ms: None,
        cancellation_requested: false,
        capture_commit_path: None,
        capture_manifest_sha256: None,
        error_code: None,
        error_message: None,
        created_at_ms: 100,
        updated_at_ms: 100,
        expires_at_ms: None,
    }
}
