import type { LiveCaptureMode, LiveSessionView } from "@/lib/app-types";

type OverlayPhase = "idle" | "initializing" | "recording" | "processing" | "feedback";
export type OverlaySurface = "collapsed" | "expanded" | Exclude<OverlayPhase, "idle"> | "success";

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

export const collapseGraceMs = 200;
export const successVisibleMs = 2_500;

// Browser preview only. Rust is the sole owner of production native-window bounds.
const previewSurfaceFrames: Record<OverlaySurface, { height: number; width: number }> = {
  collapsed: { height: 40, width: 104 },
  expanded: { height: 88, width: 180 },
  feedback: { height: 40, width: 252 },
  initializing: { height: 40, width: 112 },
  processing: { height: 40, width: 112 },
  recording: { height: 40, width: 112 },
  success: { height: 40, width: 168 },
};

export function modelFromLiveView(view: LiveSessionView): OverlayModel {
  const triggerMode = triggerModeFromCaptureMode(view.activeCaptureMode ?? view.captureMode);
  const base = {
    finalText: view.finalText ?? undefined,
    hotkey: view.hotkey,
    inputDeviceLabel: view.inputDeviceLabel ?? undefined,
    partialText: view.partialText ?? undefined,
  };
  if (view.status === "idle") {
    if (view.error) {
      return {
        ...base,
        audioLevel: 0,
        errorMessage: view.error,
        phase: "feedback",
        recordingTriggerMode: triggerMode,
      };
    }
    return { ...base, audioLevel: 0, phase: "idle", recordingTriggerMode: triggerMode };
  }

  switch (view.status) {
    case "armed":
      return { ...base, audioLevel: 0, phase: "initializing", recordingTriggerMode: triggerMode };
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

export function overlaySurface(model: OverlayModel, expanded: boolean, successVisible: boolean): OverlaySurface {
  if (model.phase !== "idle") return model.phase;
  if (successVisible) return "success";
  return expanded ? "expanded" : "collapsed";
}

export function previewOverlayFrame(surface: OverlaySurface) {
  return previewSurfaceFrames[surface];
}

function triggerModeFromCaptureMode(captureMode: LiveCaptureMode): "hold" | "toggle" {
  return captureMode === "toggle" ? "toggle" : "hold";
}
