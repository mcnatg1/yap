import type { LiveCaptureMode, LiveSessionView } from "@/lib/app-types";

type OverlayPhase = "idle" | "initializing" | "recording" | "processing" | "feedback";
export type OverlaySurface = "sensor" | "peek" | Exclude<OverlayPhase, "idle"> | "success";

export type OverlayModel = {
  audioLevel: number;
  errorMessage?: string;
  finalText?: string;
  hotkey: string;
  inputDeviceLabel?: string;
  partialText?: string;
  phase: OverlayPhase;
  recordingTriggerMode: "hold" | "toggle";
};

const compactHeight = 40;
export const hoverSensorHeight = 8;
export const idleSensorWidth = 260;
export const peekHeight = 40;
export const peekWidth = 150;
export const retractMs = 180;
export const successVisibleMs = 2_500;

const defaultWidth = 104;
const activeWidth = 112;
const successWidth = 168;
const minErrorWidth = 180;
const maxErrorWidth = 420;

export function overlayIslandWidth(surface: OverlaySurface, model: OverlayModel) {
  if (surface === "peek") return peekWidth;
  if (surface === "success") return successWidth;
  if (surface === "recording" || surface === "processing" || surface === "initializing") return activeWidth;
  if (surface === "feedback") return overlayFrame(surface, model).width;
  return defaultWidth;
}

export function modelFromLiveView(view: LiveSessionView): OverlayModel {
  const triggerMode = triggerModeFromCaptureMode(view.activeCaptureMode ?? view.captureMode);
  const base = {
    finalText: view.finalText ?? undefined,
    hotkey: view.hotkey,
    inputDeviceLabel: view.inputDeviceLabel ?? undefined,
    partialText: view.partialText ?? undefined,
  };
  if (view.status === "idle") {
    return { ...base, audioLevel: 0, phase: "idle", recordingTriggerMode: triggerMode };
  }

  switch (view.status) {
    case "armed":
      return { ...base, audioLevel: 0, phase: "recording", recordingTriggerMode: triggerMode };
    case "listening":
    case "speaking":
      return { ...base, audioLevel: view.level ?? 0, phase: "recording", recordingTriggerMode: triggerMode };
    case "settling":
    case "saving":
      return { ...base, audioLevel: 0, phase: "processing", recordingTriggerMode: triggerMode };
    case "blocked":
      return {
        ...base,
        audioLevel: 0,
        errorMessage: view.error ?? undefined,
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
  if (surface === "peek") return { height: peekHeight, width: idleSensorWidth };
  if (surface === "success") return { height: compactHeight, width: idleSensorWidth };
  if (surface === "processing") return { height: compactHeight, width: idleSensorWidth };
  if (surface === "feedback") {
    if (!model.errorMessage) return { height: compactHeight, width: defaultWidth };
    return { height: compactHeight, width: Math.min(maxErrorWidth, Math.max(minErrorWidth, model.errorMessage.length * 6.8 + 74)) };
  }
  if (surface === "recording" || surface === "initializing") return { height: compactHeight, width: idleSensorWidth };
  return { height: compactHeight, width: idleSensorWidth };
}

function triggerModeFromCaptureMode(captureMode: LiveCaptureMode): "hold" | "toggle" {
  return captureMode === "toggle" ? "toggle" : "hold";
}
