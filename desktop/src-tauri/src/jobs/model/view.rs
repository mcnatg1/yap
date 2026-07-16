use super::{
    records::RecordingJobRecord,
    status::{RecordingJobStatus, RecordingRoute, SessionMode, SessionOrigin},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineStageStatus {
    NotStarted,
    Queued,
    Running,
    Done,
    Error,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPipelineState {
    pub intake: PipelineStageStatus,
    pub preprocessing: PipelineStageStatus,
    pub transcription: PipelineStageStatus,
    pub alignment: PipelineStageStatus,
    pub diarization: PipelineStageStatus,
    pub postprocessing: PipelineStageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingJobView {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback_path: Option<String>,
    pub name: String,
    pub session_mode: SessionMode,
    pub session_origin: SessionOrigin,
    pub status: RecordingJobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RecordingRoute>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub pipeline: RecordingPipelineState,
}

impl RecordingJobView {
    pub fn from_record(record: &RecordingJobRecord) -> Self {
        Self {
            id: record.job_id.clone(),
            source_path: record
                .source_path
                .as_ref()
                .map(|path| path.display().to_string()),
            playback_path: None,
            name: record.display_name.clone(),
            session_mode: record.session_mode,
            session_origin: record.session_origin,
            status: record.status,
            route: record.route,
            output_path: record
                .output_path
                .as_ref()
                .map(|path| path.display().to_string()),
            error: record
                .error_message
                .clone()
                .or_else(|| record.error_code.clone()),
            pipeline: pipeline_for(record.status, record.route),
        }
    }
}

fn pipeline_for(
    status: RecordingJobStatus,
    route: Option<RecordingRoute>,
) -> RecordingPipelineState {
    use PipelineStageStatus as P;
    use RecordingJobStatus as S;
    let mut pipeline = RecordingPipelineState {
        intake: P::Done,
        preprocessing: P::NotStarted,
        transcription: P::NotStarted,
        alignment: P::NotStarted,
        diarization: P::NotStarted,
        postprocessing: P::NotStarted,
    };
    match status {
        S::QueuedLocalFallback => pipeline.preprocessing = P::Skipped,
        S::Preprocessing => pipeline.preprocessing = P::Running,
        S::Uploading => pipeline.preprocessing = P::Done,
        S::ServerProcessing => {
            pipeline.preprocessing = P::Done;
            pipeline.transcription = P::Running;
        }
        S::LocalTranscribing => {
            pipeline.preprocessing = P::Skipped;
            pipeline.transcription = P::Running;
        }
        S::Saving => {
            pipeline.preprocessing = completed_preprocessing(route);
            pipeline.transcription = P::Done;
            pipeline.postprocessing = P::Running;
        }
        S::DiarizationQueued => {
            pipeline.preprocessing = P::Done;
            pipeline.transcription = P::Done;
            pipeline.alignment = P::Done;
            pipeline.diarization = P::Queued;
        }
        S::DiarizationRunning => {
            pipeline.preprocessing = P::Done;
            pipeline.transcription = P::Done;
            pipeline.alignment = P::Done;
            pipeline.diarization = P::Running;
        }
        S::Complete => {
            pipeline.preprocessing = completed_preprocessing(route);
            pipeline.transcription = P::Done;
            pipeline.postprocessing = P::Done;
        }
        S::Partial => {
            pipeline.preprocessing = completed_preprocessing(route);
            pipeline.transcription = P::Done;
        }
        S::Failed => pipeline.preprocessing = completed_preprocessing(route),
        S::Accepted
        | S::Preflighting
        | S::BlockedSetupRequired
        | S::BlockedServerUnavailable
        | S::BlockedSignInRequired
        | S::QueuedServer
        | S::Cancelled => {}
    }
    pipeline
}

fn completed_preprocessing(route: Option<RecordingRoute>) -> PipelineStageStatus {
    match route {
        Some(RecordingRoute::LocalFallback) => PipelineStageStatus::Skipped,
        Some(RecordingRoute::ServerBatch | RecordingRoute::ServerLive) => PipelineStageStatus::Done,
        None => PipelineStageStatus::NotStarted,
    }
}
