import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { LiveCaptureMode, LiveInputDeviceView, LiveSessionView, WorkspaceView } from "@/lib/app-types";

export type SavedLiveSession = {
  createdAtMs: number;
  name: string;
  sourcePath: string;
  outputPath: string;
  warning?: string | null;
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

export function setLivePasteHotkey(hotkey: string): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("set_live_paste_hotkey", { hotkey });
}

export function clearLivePasteHotkey(): Promise<LiveSessionView> {
  return invoke<LiveSessionView>("clear_live_paste_hotkey");
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

export function listSavedLiveSessions(): Promise<SavedLiveSession[]> {
  return invoke<SavedLiveSession[]>("list_saved_live_sessions");
}

export function showMainWorkspace(workspace: WorkspaceView): Promise<void> {
  return invoke<void>("show_main_workspace", { workspace });
}

export async function listenLiveSession(onUpdate: (view: LiveSessionView) => void): Promise<UnlistenFn> {
  if (!isTauri()) return () => undefined;
  return listen<LiveSessionView>("live-session", (event) => onUpdate(event.payload));
}
