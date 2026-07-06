import type { LiveCaptureMode, LiveSessionView } from "@/lib/app-types";

export type OverlayPhase = "idle" | "initializing" | "recording" | "processing" | "feedback" | "updateAvailable";
export type OverlaySurface = "sensor" | "peek" | Exclude<OverlayPhase, "idle"> | "success";

export type OverlayModel = {
  audioLevel: number;
  errorMessage?: string;
  finalText?: string;
  hotkey: string;
  inputDeviceLabel?: string;
  isCommandMode: boolean;
  partialText?: string;
  phase: OverlayPhase;
  recordingTriggerMode: "hold" | "toggle";
  updateVersion?: string;
};

export const compactHeight = 40;
export const hoverSensorHeight = 8;
export const idleSensorWidth = 260;
export const peekHeight = 40;
export const peekWidth = 150;
export const retractMs = 180;
export const successVisibleMs = 10_000;

const defaultWidth = 92;
const processingWidth = 92;
const successWidth = 168;
const toggleWidth = 104;
const commandModeWidth = 180;
const updateWidth = 190;
const minErrorWidth = 180;
const maxErrorWidth = 420;

export function modelFromLiveView(view: LiveSessionView): OverlayModel {
  const triggerMode = triggerModeFromCaptureMode(view.captureMode);
  const base = {
    finalText: view.finalText,
    hotkey: view.hotkey,
    inputDeviceLabel: view.inputDeviceLabel,
    partialText: view.partialText,
  };
  if (view.visibility === "hidden" || view.status === "idle") {
    return { ...base, audioLevel: 0, isCommandMode: false, phase: "idle", recordingTriggerMode: triggerMode };
  }

  switch (view.status) {
    case "armed":
      return { ...base, audioLevel: 0, isCommandMode: false, phase: "recording", recordingTriggerMode: triggerMode };
    case "listening":
    case "speaking":
      return { ...base, audioLevel: view.level ?? 0, isCommandMode: false, phase: "recording", recordingTriggerMode: triggerMode };
    case "settling":
    case "saving":
      return { ...base, audioLevel: 0, isCommandMode: false, phase: "processing", recordingTriggerMode: triggerMode };
    case "blocked":
      return {
        ...base,
        audioLevel: 0,
        errorMessage: view.error,
        isCommandMode: false,
        phase: "feedback",
        recordingTriggerMode: triggerMode,
      };
  }
}

export function overlaySurface(model: OverlayModel, peeked: boolean, retracting: boolean, successVisible: boolean): OverlaySurface {
  if (model.phase !== "idle") return model.phase;
  if (successVisible) return "success";
  if (peeked || retracting) return "peek";
  return "sensor";
}

export function overlayFrame(surface: OverlaySurface, model: OverlayModel) {
  if (surface === "sensor") return { height: hoverSensorHeight, width: idleSensorWidth };
  if (surface === "peek") return { height: peekHeight, width: peekWidth };
  if (surface === "success") return { height: compactHeight, width: successWidth };
  if (surface === "processing") return { height: compactHeight, width: processingWidth };
  if (surface === "feedback") {
    if (!model.errorMessage) return { height: compactHeight, width: defaultWidth };
    return { height: compactHeight, width: Math.min(maxErrorWidth, Math.max(minErrorWidth, model.errorMessage.length * 6.8 + 74)) };
  }
  if (surface === "updateAvailable") return { height: compactHeight, width: updateWidth };
  if (model.isCommandMode) return { height: compactHeight, width: commandModeWidth };
  if (surface === "recording" && model.recordingTriggerMode === "toggle") return { height: compactHeight, width: toggleWidth };
  return { height: compactHeight, width: defaultWidth };
}

function triggerModeFromCaptureMode(captureMode: LiveCaptureMode): "hold" | "toggle" {
  return captureMode === "toggle" ? "toggle" : "hold";
}
