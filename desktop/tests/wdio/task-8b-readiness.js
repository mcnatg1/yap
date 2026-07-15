export const MICROPHONE_PERMISSION_DENIED_PREFIX = "Microphone permission denied:";

function isNarrowPermissionDenial(error) {
  return typeof error === "string"
    && error.startsWith(MICROPHONE_PERMISSION_DENIED_PREFIX)
    && error.slice(MICROPHONE_PERMISSION_DENIED_PREFIX.length).trim().length > 0;
}

export function classifyNativeReadiness(environment) {
  const modelStatus = environment?.model?.status;
  const modelSkipStatuses = ["missing", "disabled", "corrupted"];
  if (modelStatus !== "ready" && !modelSkipStatuses.includes(modelStatus)) {
    throw new Error(`Unexpected Nemotron model status: ${modelStatus ?? "unknown"}`);
  }

  if (environment.deviceError) {
    if (isNarrowPermissionDenial(environment.deviceError)) {
      return {
        action: "skip",
        reason: `microphone permission was denied during enumeration: ${environment.deviceError}`,
      };
    }
    throw new Error(`Input device enumeration failed: ${environment.deviceError}`);
  }
  if (!Array.isArray(environment.devices)) {
    throw new Error("Input device enumeration failed: native command returned no device list");
  }
  if (environment.devices.length === 0) {
    return { action: "skip", reason: "no input device was enumerated" };
  }

  const preflight = environment.preflight;
  if (!preflight || typeof preflight.status !== "string") {
    throw new Error("Unexpected microphone preflight failure: native command returned no status");
  }
  if (preflight.status === "blocked") {
    const error = preflight.error;
    if (isNarrowPermissionDenial(error)) {
      return { action: "skip", reason: `microphone preflight permission was denied: ${error}` };
    }
    throw new Error(`Unexpected microphone preflight failure: ${error || "unknown error"}`);
  }
  if (preflight.status !== "idle") {
    throw new Error(`Unexpected microphone preflight status: ${preflight.status}`);
  }
  if (modelSkipStatuses.includes(modelStatus)) {
    return { action: "skip", reason: `Nemotron model is ${modelStatus}` };
  }
  return { action: "run" };
}
