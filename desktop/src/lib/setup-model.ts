export type SetupState =
  | "checking"
  | "fallback_missing"
  | "fallback_installing"
  | "fallback_ready"
  | "fallback_disabled"
  | "setup_error";

export type FallbackModelStatus =
  | "missing"
  | "downloading"
  | "verifying"
  | "ready"
  | "corrupted"
  | "disabled"
  | "error";

export type FallbackModelView = {
  id: "nemotron-3.5-asr-streaming-0.6b-1120ms-int8";
  label: string;
  status: FallbackModelStatus;
  installedBytes?: number | null;
  totalBytes?: number | null;
  progressPercent?: number | null;
  speedMbps?: number | null;
  message?: string | null;
  modelsDir: string;
};

export type ServerConnectionState =
  | "not_set"
  | "connecting"
  | "ready"
  | "offline"
  | "sign_in_required"
  | "retrying"
  | "disabled";

export type LocalComputeTargetView = {
  id: string;
  label: string;
  selected: boolean;
};

export type SetupSnapshot = {
  engineReady: boolean;
  fallbackEnabled: boolean;
  modelInstalled: boolean;
};

export function deriveSetupState(snapshot: SetupSnapshot): SetupState {
  const status: FallbackModelStatus = snapshot.engineReady && snapshot.modelInstalled
    ? "ready"
    : snapshot.modelInstalled
      ? "corrupted"
      : "missing";
  return deriveSetupStateFromFallbackModel(status, snapshot.fallbackEnabled);
}

export function deriveSetupStateFromFallbackModel(
  status: FallbackModelStatus,
  fallbackEnabled: boolean,
): SetupState {
  if (!fallbackEnabled || status === "disabled") return "fallback_disabled";

  switch (status) {
    case "downloading":
    case "verifying":
      return "fallback_installing";
    case "ready":
      return "fallback_ready";
    case "error":
      return "setup_error";
    case "missing":
    case "corrupted":
      return "fallback_missing";
  }
}

export function isFallbackModelBusy(
  fallbackModel?: Pick<FallbackModelView, "status"> | null,
  pending = false,
) {
  return pending || fallbackModel?.status === "downloading" || fallbackModel?.status === "verifying";
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

export function fallbackModelLabel(model: string) {
  const normalized = model.toLowerCase();
  if (normalized.includes("nemotron")) {
    return "Nemotron 3.5 ASR Streaming 0.6B INT8";
  }
  return model.replace("cstr/", "").replace(/\.(gguf|onnx)$/i, "");
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
