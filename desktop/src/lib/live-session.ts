type LiveOverlayVisibility = "enabled" | "hidden";

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
  activeCaptureMode?: LiveCaptureMode | null;
  hotkey: string;
  pasteHotkey: string;
  inputDeviceId?: string | null;
  inputDeviceLabel?: string | null;
  level?: number | null;
  partialText?: string | null;
  finalText?: string | null;
  transcriptionDegraded?: boolean;
  error?: string | null;
};

export type LiveOverlayView = {
  visibility: LiveOverlayVisibility;
  status: LiveSessionStatus;
  captureMode: LiveCaptureMode;
  activeCaptureMode?: LiveCaptureMode | null;
  level?: number | null;
  hasFinalText: boolean;
  error?: string | null;
};

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
