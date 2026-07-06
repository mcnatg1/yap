export const workspaceViews = ["home", "transcribe", "polish"] as const;

export type WorkspaceView = (typeof workspaceViews)[number];

export type RailAction = WorkspaceView | "details" | "help";

export const workspaceCopy: Record<WorkspaceView, { title: string; description: string }> = {
  home: {
    title: "Welcome back",
    description: "Past recordings.",
  },
  transcribe: {
    title: "Transcribe",
    description: "Add files. Run when ready.",
  },
  polish: {
    title: "Polish",
    description: "Clean text.",
  },
};

export function isWorkspaceView(value: unknown): value is WorkspaceView {
  return typeof value === "string" && (workspaceViews as readonly string[]).includes(value);
}

export const acceptedFormats = "MP3, M4A, WAV, MP4, FLAC, OGG, WEBM";

export const audioExtensions = ["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];
export const audioExts = new Set(audioExtensions.map((format) => `.${format}`));

export type SetupState =
  | "checking"
  | "fallback_missing"
  | "fallback_installing"
  | "fallback_ready"
  | "fallback_disabled"
  | "setup_error";

export type ServerConnectionState =
  | "not_set"
  | "connecting"
  | "ready"
  | "offline"
  | "sign_in_required"
  | "retrying"
  | "disabled";

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
  | "server_processing_cohere"
  | "local_transcribing"
  | "saving"
  | "diarization_queued"
  | "diarization_running"
  | "complete"
  | "partial"
  | "failed"
  | "cancelled";

export type RecordingIntent = "live" | "recording";
export type RecordingRoute = "localFallback" | "serverBatch" | "serverLive";
export type PipelineStageStatus = "notStarted" | "queued" | "running" | "done" | "error" | "skipped";
export type LiveOverlayVisibility = "enabled" | "hidden";
export type LiveCaptureMode = "pushToTalk" | "toggle";
export type LiveSessionStatus =
  | "idle"
  | "armed"
  | "listening"
  | "speaking"
  | "settling"
  | "blocked"
  | "saving";
export type LiveRoute = "serverLive" | "localFallback" | "blocked" | "none";

export type LiveInputDeviceView = {
  id: string;
  label: string;
  isDefault: boolean;
  selected: boolean;
};

export type LiveSessionView = {
  visibility: LiveOverlayVisibility;
  status: LiveSessionStatus;
  route: LiveRoute;
  captureMode: LiveCaptureMode;
  hotkey: string;
  inputDeviceId?: string;
  inputDeviceLabel?: string;
  level?: number;
  partialText?: string;
  finalText?: string;
  error?: string;
};

export type RecordingPipelineState = {
  intake: PipelineStageStatus;
  preprocessing: PipelineStageStatus;
  transcription: PipelineStageStatus;
  alignment: PipelineStageStatus;
  diarization: PipelineStageStatus;
  postprocessing: PipelineStageStatus;
};

export type RecordingJobView = {
  id: number;
  path: string;
  name: string;
  intent: RecordingIntent;
  status: RecordingJobStatus;
  route?: RecordingRoute;
  output?: string;
  error?: string;
  progressPhase?: string;
  progressPercent?: number;
  progressMessage?: string;
  pipeline: RecordingPipelineState;
};

export type SetupSnapshot = {
  engineReady: boolean;
  fallbackEnabled: boolean;
  modelInstalled: boolean;
};

const activeRecordingStatuses = new Set<RecordingJobStatus>([
  "preflighting",
  "preprocessing",
  "uploading",
  "server_processing_cohere",
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

export function deriveSetupState(snapshot: SetupSnapshot): SetupState {
  if (!snapshot.fallbackEnabled) return "fallback_disabled";
  if (snapshot.engineReady && snapshot.modelInstalled) return "fallback_ready";
  return "fallback_missing";
}

export function isRecordingActive(status: RecordingJobStatus) {
  return activeRecordingStatuses.has(status);
}

export function isRecordingFinished(status?: RecordingJobStatus) {
  return status ? finishedRecordingStatuses.has(status) : false;
}

export function isRecordingRunnable(status: RecordingJobStatus) {
  return status === "queued_local_fallback" || status === "failed";
}

export function isRecordingRetryable(status: RecordingJobStatus) {
  return (
    status === "failed" ||
    status === "blocked_sign_in_required"
  );
}

export function recordingStatusForStartFailure(code?: string): RecordingJobStatus {
  switch (code) {
    case "MODEL_MISSING":
    case "FALLBACK_DISABLED":
      return "blocked_setup_required";
    case "SERVER_UNAVAILABLE":
    case "SERVER_OFFLINE":
      return "blocked_server_unavailable";
    case "SIGN_IN_REQUIRED":
      return "blocked_sign_in_required";
    default:
      return "failed";
  }
}

export function setupStateLabel(state: SetupState) {
  switch (state) {
    case "checking":
      return "Checking";
    case "fallback_missing":
      return "Setup";
    case "fallback_installing":
      return "Installing";
    case "fallback_ready":
      return "Ready";
    case "fallback_disabled":
      return "Disabled";
    case "setup_error":
      return "Needs attention";
  }
}

export function serverConnectionLabel(state: ServerConnectionState) {
  switch (state) {
    case "not_set":
      return "Not set";
    case "connecting":
      return "Checking";
    case "ready":
      return "Ready";
    case "offline":
      return "Offline";
    case "sign_in_required":
      return "Sign in";
    case "retrying":
      return "Retrying";
    case "disabled":
      return "Disabled";
  }
}

export function liveRouteLabel(route: LiveRoute) {
  switch (route) {
    case "serverLive":
      return "Server";
    case "localFallback":
      return "Local fallback";
    case "blocked":
      return "Needs setup";
    case "none":
      return "Idle";
  }
}

export function liveStatusLabel(status: LiveSessionStatus) {
  switch (status) {
    case "idle":
      return "Idle";
    case "armed":
      return "Armed";
    case "listening":
      return "Listening";
    case "speaking":
      return "Speaking";
    case "settling":
      return "Settling";
    case "blocked":
      return "Blocked";
    case "saving":
      return "Saving";
  }
}

export function basename(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

export function extension(path: string) {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

export function formatHistoryDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Saved";

  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    month: "short",
  }).format(date);
}

function localDayKey(date: Date) {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

export function historyEntryTime(entry: { createdAt: string }) {
  const time = Date.parse(entry.createdAt);
  return Number.isFinite(time) ? time : 0;
}

function historyDayLabel(date: Date) {
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  const key = localDayKey(date);
  if (key === localDayKey(today)) return "Today";
  if (key === localDayKey(yesterday)) return "Yesterday";
  return new Intl.DateTimeFormat(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
  }).format(date);
}

export function formatElapsed(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return `${minutes}:${String(remainder).padStart(2, "0")}`;
}

export function formatHistoryTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Saved";

  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

export function groupHistoryByDay<T extends { createdAt: string }>(entries: T[]) {
  const sorted = [...entries].sort((a, b) => historyEntryTime(b) - historyEntryTime(a));
  const groups: { key: string; label: string; entries: T[] }[] = [];
  const indexByKey = new Map<string, number>();

  for (const entry of sorted) {
    const date = new Date(entry.createdAt);
    const key = Number.isNaN(date.getTime()) ? "unknown" : localDayKey(date);
    const label = Number.isNaN(date.getTime()) ? "Earlier" : historyDayLabel(date);
    const index = indexByKey.get(key);

    if (index === undefined) {
      indexByKey.set(key, groups.length);
      groups.push({ key, label, entries: [entry] });
    } else {
      groups[index].entries.push(entry);
    }
  }

  return groups;
}
