import { isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

import type { LiveCaptureMode, LiveInputDeviceView, LiveSessionView } from "@/lib/live-session";
import {
  clearLiveHotkey,
  clearLivePasteHotkey,
  listInputDevices,
  listenLiveSession,
  liveStatus,
  preflightInputDevice,
  recordLiveHotkey,
  recordLivePasteHotkey,
  resetLiveHotkey as resetLiveHotkeyToDefault,
  resetLivePasteHotkey as resetLivePasteHotkeyToDefault,
  setInputDevice,
  setLiveCaptureMode,
  setLiveOverlayEnabled,
  startLiveSession,
  stopLiveSession,
} from "@/live";

const initialLiveView: LiveSessionView = {
  captureMode: "pushToTalk",
  hotkey: "Ctrl+Shift+Space",
  pasteHotkey: "Ctrl+Shift+Alt+V",
  route: "none",
  status: "idle",
  transcriptionDegraded: false,
  visibility: "enabled",
};

export function useLiveControl() {
  const [liveView, setLiveView] = useState<LiveSessionView>(initialLiveView);
  const [liveInputDevices, setLiveInputDevices] = useState<LiveInputDeviceView[]>([]);
  const [liveBusy, setLiveBusy] = useState(false);
  const [liveSettingsError, setLiveSettingsError] = useState("");

  const refreshLiveState = useCallback(async () => {
    if (!isTauri()) return;

    const [live, devices] = await Promise.all([liveStatus(), listInputDevices()]);
    setLiveView(live);
    setLiveInputDevices(devices);
  }, []);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlistenLive: (() => void) | undefined;
    void listenLiveSession(setLiveView).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLive = stop;
    });

    return () => {
      cancelled = true;
      unlistenLive?.();
    };
  }, []);

  const updateLive = useCallback(
    async (action: () => Promise<LiveSessionView>, message?: string) => {
      if (!isTauri() || liveBusy) return;

      setLiveBusy(true);
      try {
        setLiveSettingsError("");
        const view = await action();
        setLiveView(view);
        setLiveInputDevices(await listInputDevices());
        if (message) toast.success(message);
      } catch (error) {
        const nextMessage = String(error);
        setLiveSettingsError(nextMessage);
        toast.error(nextMessage);
      } finally {
        setLiveBusy(false);
      }
    },
    [liveBusy],
  );

  const updateLiveOverlay = useCallback(
    (enabled: boolean) => {
      void updateLive(() => setLiveOverlayEnabled(enabled), enabled ? "Live overlay enabled" : "Live overlay hidden");
    },
    [updateLive],
  );

  const updateLiveHotkey = useCallback(
    () => updateLive(recordLiveHotkey, "Live shortcut updated"),
    [updateLive],
  );

  const resetLiveHotkey = useCallback(() => {
    void updateLive(resetLiveHotkeyToDefault, "Live shortcut reset");
  }, [updateLive]);

  const clearLiveShortcut = useCallback(() => {
    void updateLive(clearLiveHotkey, "Live shortcut cleared");
  }, [updateLive]);

  const updateLivePasteHotkey = useCallback(
    () => updateLive(recordLivePasteHotkey, "Paste shortcut updated"),
    [updateLive],
  );

  const clearLivePasteShortcut = useCallback(() => {
    void updateLive(clearLivePasteHotkey, "Paste shortcut cleared");
  }, [updateLive]);

  const resetLivePasteHotkey = useCallback(() => {
    void updateLive(resetLivePasteHotkeyToDefault, "Paste shortcut reset");
  }, [updateLive]);

  const updateLiveCaptureMode = useCallback(
    (captureMode: LiveCaptureMode) => {
      void updateLive(() => setLiveCaptureMode(captureMode));
    },
    [updateLive],
  );

  const updateInputDevice = useCallback(
    (deviceId?: string) => {
      void updateLive(() => setInputDevice(deviceId));
    },
    [updateLive],
  );

  const preflightLiveInput = useCallback(() => {
    void updateLive(preflightInputDevice);
  }, [updateLive]);

  const startLive = useCallback(() => {
    void updateLive(startLiveSession);
  }, [updateLive]);

  const stopLive = useCallback(() => {
    void updateLive(stopLiveSession);
  }, [updateLive]);

  return {
    clearLivePasteShortcut,
    clearLiveShortcut,
    liveBusy,
    liveInputDevices,
    liveSettingsError,
    liveView,
    preflightLiveInput,
    refreshLiveState,
    resetLiveHotkey,
    resetLivePasteHotkey,
    startLive,
    stopLive,
    updateInputDevice,
    updateLiveCaptureMode,
    updateLiveHotkey,
    updateLiveOverlay,
    updateLivePasteHotkey,
  };
}
