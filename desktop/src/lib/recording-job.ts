export const queuedServerMessage =
  "Queued for your organization's transcription server. It will start when Yap connects.";

export type RecordingJobStatus =
  | "accepted"
  | "preflighting"
  | "blocked_setup_required"
  | "blocked_server_unavailable"
  | "blocked_sign_in_required"
  | "queued_local_fallback"
  | "queued_server"
  | "preprocessing"
  | "uploading"
  | "server_processing"
  | "local_transcribing"
  | "saving"
  | "diarization_queued"
  | "diarization_running"
  | "complete"
  | "partial"
  | "failed"
  | "cancelled";

type RecordingRoute = "localFallback" | "serverBatch" | "serverLive";
type PipelineStageStatus = "notStarted" | "queued" | "running" | "done" | "error" | "skipped";

export type RecordingPipelineState = {
  intake: PipelineStageStatus;
  preprocessing: PipelineStageStatus;
  transcription: PipelineStageStatus;
  alignment: PipelineStageStatus;
  diarization: PipelineStageStatus;
  postprocessing: PipelineStageStatus;
};

export type PlaybackAdmission = {
  playbackPath: string;
  byteLength: number;
};

export type RecordingJobView = {
  id: string;
  sourcePath?: string;
  playbackPath?: string;
  outputPath?: string;
  error?: string;
  name: string;
  sessionMode: "dictation" | "meeting";
  sessionOrigin: "liveCapture" | "importedFile";
  status: RecordingJobStatus;
  route?: RecordingRoute;
  pipeline: RecordingPipelineState;
};

const activeRecordingStatuses = new Set<RecordingJobStatus>([
  "preflighting",
  "preprocessing",
  "uploading",
  "server_processing",
  "local_transcribing",
  "saving",
  "diarization_running",
]);

const finishedRecordingStatuses = new Set<RecordingJobStatus>(["complete", "partial"]);

export function createInitialPipelineState(): RecordingPipelineState {
  return {
    intake: "queued",
    preprocessing: "notStarted",
    transcription: "notStarted",
    alignment: "notStarted",
    diarization: "notStarted",
    postprocessing: "notStarted",
  };
}

export function isRecordingActive(status: RecordingJobStatus) {
  return activeRecordingStatuses.has(status);
}

export function isRecordingCancellable(status: RecordingJobStatus) {
  return status === "accepted" ||
    status === "blocked_setup_required" ||
    status === "blocked_server_unavailable" ||
    status === "blocked_sign_in_required" ||
    status === "queued_local_fallback" ||
    status === "queued_server" ||
    status === "preprocessing" ||
    status === "uploading" ||
    status === "server_processing" ||
    status === "saving";
}

export function isRecordingFinished(status?: RecordingJobStatus) {
  return status ? finishedRecordingStatuses.has(status) : false;
}
