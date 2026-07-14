import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { LiveCaptureMode, LiveInputDeviceView, LiveSessionStatus, LiveSessionView, WorkspaceView } from "@/lib/app-types";

export type SavedLiveSession = {
  captureCommitPath?: string | null;
  createdAtMs: number;
  name: string;
  sourcePath: string;
  outputPath: string;
  sessionId: string;
  warning?: string | null;
  recoveryState?: "recoverable" | "recovered" | null;
};

export type SavedLiveSessionCatalog = {
  maintenanceWarnings: string[];
  sessions: SavedLiveSession[];
};

export type OwnedLiveTranscriptPathResolution = {
  requestedPath: string;
  canonicalPath?: string | null;
  missing: boolean;
};

export type RecoverableLiveSession = {
  audioPartialPath?: string | null;
  expiresAtMs: number;
  journalPartialPath?: string | null;
  name: string;
  reason: string;
  sessionId: string;
};

export type LiveLevelView = {
  level?: number | null;
  status: LiveSessionStatus;
};

export function liveStatus(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("live_status");
}

export function showLiveOverlay(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("show_live_overlay");
}

export function setLiveOverlayEnabled(enabled: boolean): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("set_live_overlay_enabled", { enabled });
}

export function setLiveHotkey(hotkey: string): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("set_live_hotkey", { hotkey });
}

export function clearLiveHotkey(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("clear_live_hotkey");
}

export function resetLiveHotkey(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("reset_live_hotkey");
}

export function setLivePasteHotkey(hotkey: string): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("set_live_paste_hotkey", { hotkey });
}

export function clearLivePasteHotkey(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("clear_live_paste_hotkey");
}

export function resetLivePasteHotkey(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("reset_live_paste_hotkey");
}

export function setLiveCaptureMode(captureMode: LiveCaptureMode): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("set_live_capture_mode", { captureMode });
}

export function listInputDevices(): Promise<LiveInputDeviceView[]> {
  return invoke<LiveInputDeviceView[]>("list_input_devices");
}

export function setInputDevice(deviceId?: string): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("set_input_device", { deviceId });
}

export function preflightInputDevice(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("preflight_input_device");
}

export function startLiveSession(activeCaptureMode?: LiveCaptureMode): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("start_live_session", { activeCaptureMode });
}

export function stopLiveSession(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("stop_live_session");
}

export function listSavedLiveSessions(): Promise<SavedLiveSessionCatalog> {
  return invoke<SavedLiveSessionCatalog>("list_saved_live_sessions");
}

export function listRecoverableLiveSessions(): Promise<RecoverableLiveSession[]> {
  return invoke<RecoverableLiveSession[]>("list_recoverable_live_sessions");
}

export function recoverLiveSession(
  sessionId: string,
  expectedArtifactPath: string,
): Promise<SavedLiveSession> {
  return invoke<SavedLiveSession>("recover_live_session", { expectedArtifactPath, sessionId });
}

export function deleteRecoverableLiveSession(
  sessionId: string,
  expectedArtifactPath: string,
): Promise<void> {
  return invoke<void>("delete_recoverable_live_session", { expectedArtifactPath, sessionId });
}

export function deleteSavedLiveSession(
  sessionId: string,
  expectedOutputPath: string,
  expectedCaptureCommitPath: string,
): Promise<void> {
  return invoke<void>("delete_saved_live_session", {
    expectedCaptureCommitPath,
    expectedOutputPath,
    sessionId,
  });
}

export function resolveOwnedLiveTranscriptPaths(
  outputPaths: string[],
): Promise<OwnedLiveTranscriptPathResolution[]> {
  if (!isTauri()) return Promise.resolve([]);
  return invoke<OwnedLiveTranscriptPathResolution[]>("resolve_owned_live_transcript_paths", {
    outputPaths,
  });
}

export function showMainWorkspace(workspace: WorkspaceView): Promise<void> {
  return invoke<void>("show_main_workspace", { workspace });
}

export async function listenLiveSession(onUpdate: (view: LiveSessionView) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<LiveSessionView>("live-session", (event) => onUpdate(event.payload));
}

export async function listenLiveLevel(onUpdate: (view: LiveLevelView) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<LiveLevelView>("live-level", (event) => onUpdate(event.payload));
}

export async function listenLiveSessionSaved(
  onSaved: (session: SavedLiveSession) => void,
): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<SavedLiveSession>("live-session-saved", (event) => onSaved(event.payload));
}
