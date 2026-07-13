import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { useLiveControl } from "@/hooks/use-live-control";
import { useLocalComputeTargets } from "@/hooks/use-local-compute-targets";
import { useServerConnection } from "@/hooks/use-server-connection";
import {
  isFallbackModelBusy,
  type FallbackModelView,
} from "@/lib/app-types";
import {
  projectFallbackModelState,
  type FallbackModelStateOverrides,
} from "@/lib/setup-model-state";
import {
  cancelFallbackModelInstall,
  fallbackModelStatus,
  installFallbackModel,
  listenFallbackModelProgress,
  listenFallbackModelStatus,
  openFallbackModelFolder,
  removeFallbackModel,
  setFallbackModelEnabled,
  verifyFallbackModel,
} from "@/settings";

type SetupStatus = {
  model: string;
  root: string;
  engineReady: boolean;
  engineBinaryStatus: string;
  fallbackEnabled: boolean;
  modelInstalled: boolean;
  engineStatus: string;
};

const setupSkipKey = "yap-local-fallback-setup-skipped";

export function useSettingsControl({
  onStatusChange,
}: {
  onStatusChange: (status: string) => void;
}) {
  const [auth, setAuth] = useState("Checking");
  const [, setEngineReady] = useState(false);
  const [fallbackEnabled, setFallbackEnabled] = useState(true);
  const [fallbackModel, setFallbackModel] = useState<FallbackModelView | null>(null);
  const [modelInstalled, setModelInstalled] = useState(false);
  const [fallbackCommandPending, setFallbackCommandPending] = useState(false);
  const [setupPromptRequest, setSetupPromptRequest] = useState(false);
  const setupPromptedRef = useRef(false);
  const fallbackEnabledRef = useRef(fallbackEnabled);
  const modelInstalledRef = useRef(modelInstalled);
  const callbacksRef = useRef({ onStatusChange });
  callbacksRef.current = { onStatusChange };

  const { refreshServerState, serverLabel } = useServerConnection();
  const live = useLiveControl();
  const fallbackModelBusy = isFallbackModelBusy(fallbackModel, fallbackCommandPending);
  const compute = useLocalComputeTargets(fallbackModelBusy);
  const refreshPortsRef = useRef({
    loadComputeTargets: compute.loadComputeTargets,
    refreshLiveState: live.refreshLiveState,
    refreshServerState,
  });
  refreshPortsRef.current = {
    loadComputeTargets: compute.loadComputeTargets,
    refreshLiveState: live.refreshLiveState,
    refreshServerState,
  };

  useEffect(() => {
    fallbackEnabledRef.current = fallbackEnabled;
  }, [fallbackEnabled]);

  useEffect(() => {
    modelInstalledRef.current = modelInstalled;
  }, [modelInstalled]);

  const applyFallbackModelView = useCallback((
    view: FallbackModelView,
    overrides: FallbackModelStateOverrides = {},
  ) => {
    const projection = projectFallbackModelState({
      alreadyPrompted: setupPromptedRef.current,
      currentFallbackEnabled: fallbackEnabledRef.current,
      currentModelInstalled: modelInstalledRef.current,
      overrides,
      skipped: localStorage.getItem(setupSkipKey) === "true",
      view,
    });

    fallbackEnabledRef.current = projection.fallbackEnabled;
    modelInstalledRef.current = projection.modelInstalled;
    setupPromptedRef.current = projection.setupPrompted;
    setFallbackModel(view);
    callbacksRef.current.onStatusChange(projection.status);
    setAuth(projection.auth);
    setEngineReady(projection.engineReady);
    setFallbackEnabled(projection.fallbackEnabled);
    setModelInstalled(projection.modelInstalled);
    if (projection.requestSetupPrompt) setSetupPromptRequest(true);
  }, []);

  const applySetupStatus = useCallback((setup: SetupStatus) => {
    fallbackEnabledRef.current = setup.fallbackEnabled;
    modelInstalledRef.current = setup.modelInstalled;
    callbacksRef.current.onStatusChange(setup.engineReady ? setup.engineStatus : "Setup");
    setAuth(setup.engineReady ? "Ready" : "Setup");
    setEngineReady(setup.engineReady);
    setFallbackEnabled(setup.fallbackEnabled);
    setModelInstalled(setup.modelInstalled);
  }, []);

  const refresh = useCallback(async () => {
    if (!isTauri()) return;

    try {
      const [setup, view] = await Promise.all([
        invoke<SetupStatus>("setup_status"),
        fallbackModelStatus(),
        refreshPortsRef.current.refreshServerState(),
      ]);
      applySetupStatus(setup);
      applyFallbackModelView(view, {
        authText: setup.engineReady ? "Ready" : "Setup",
        engineReady: setup.engineReady,
        fallbackEnabled: setup.fallbackEnabled,
        modelInstalled: setup.modelInstalled,
      });
      await Promise.all([
        refreshPortsRef.current.refreshLiveState(),
        refreshPortsRef.current.loadComputeTargets(),
      ]);
    } catch (error) {
      callbacksRef.current.onStatusChange("Setup check failed");
      setAuth(String(error));
    }
  }, [applyFallbackModelView, applySetupStatus]);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlistenFallbackProgress: (() => void) | undefined;
    let unlistenFallbackStatus: (() => void) | undefined;

    void listenFallbackModelProgress(applyFallbackModelView).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenFallbackProgress = stop;
    });

    void listenFallbackModelStatus(applyFallbackModelView).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenFallbackStatus = stop;
    });

    return () => {
      cancelled = true;
      unlistenFallbackProgress?.();
      unlistenFallbackStatus?.();
    };
  }, [applyFallbackModelView]);

  useEffect(() => {
    void refresh();

    if (!isTauri()) {
      callbacksRef.current.onStatusChange("Preview");
      setAuth("Tauri bridge");
    }
  }, [refresh]);

  const installFallback = useCallback(async (options: { force?: boolean } = {}) => {
    if (!isTauri() || fallbackModelBusy) return;

    setFallbackCommandPending(true);
    fallbackEnabledRef.current = true;
    setFallbackEnabled(true);
    callbacksRef.current.onStatusChange("Installing local fallback");
    try {
      const view = await installFallbackModel({ force: options.force });
      localStorage.removeItem(setupSkipKey);
      applyFallbackModelView(view, { fallbackEnabled: true });
      if (view.status === "ready") {
        toast.success(options.force ? "Local fallback reinstalled" : "Local fallback installed");
      } else {
        toast.info(view.message ?? "Local fallback install did not complete");
      }
    } catch (error) {
      toast.error(`Install failed: ${String(error)}`);
      await refresh();
    } finally {
      setFallbackCommandPending(false);
    }
  }, [applyFallbackModelView, fallbackModelBusy, refresh]);

  const removeFallback = useCallback(async () => {
    if (!isTauri() || fallbackModelBusy) return;

    setFallbackCommandPending(true);
    try {
      localStorage.setItem(setupSkipKey, "true");
      const view = await removeFallbackModel();
      applyFallbackModelView(view, {
        engineReady: false,
        fallbackEnabled: false,
        modelInstalled: false,
      });
      toast.success("Local fallback files removed");
    } catch (error) {
      toast.error(`Remove failed: ${String(error)}`);
      await refresh();
    } finally {
      setFallbackCommandPending(false);
    }
  }, [applyFallbackModelView, fallbackModelBusy, refresh]);

  const setFallbackEnabledSetting = useCallback(async (enabled: boolean) => {
    if (!isTauri() || fallbackModelBusy) return;

    setFallbackCommandPending(true);
    try {
      const view = await setFallbackModelEnabled(enabled);
      if (!enabled) localStorage.setItem(setupSkipKey, "true");
      applyFallbackModelView(view, {
        engineReady: enabled && view.status === "ready",
        fallbackEnabled: enabled,
        modelInstalled: enabled && view.status === "missing" ? false : modelInstalledRef.current,
      });
      toast.success(enabled ? "Local fallback enabled" : "Local fallback disabled");
    } catch (error) {
      toast.error(`Update failed: ${String(error)}`);
      await refresh();
    } finally {
      setFallbackCommandPending(false);
    }
  }, [applyFallbackModelView, fallbackModelBusy, refresh]);

  const cancelFallbackInstall = useCallback(async () => {
    if (!isTauri() || fallbackModel?.status !== "downloading") return;
    setFallbackCommandPending(true);
    try {
      const view = await cancelFallbackModelInstall();
      applyFallbackModelView(view, { fallbackEnabled: true });
      if (view.status !== "missing" && view.status !== "error") {
        applyFallbackModelView(await fallbackModelStatus(), { fallbackEnabled: true });
      }
      toast.success("Local fallback cancellation requested");
    } catch (error) {
      toast.error(`Cancel failed: ${String(error)}`);
      await refresh();
    } finally {
      setFallbackCommandPending(false);
    }
  }, [applyFallbackModelView, fallbackModel?.status, refresh]);

  const verifyFallback = useCallback(async () => {
    if (!isTauri() || fallbackModelBusy) return;

    setFallbackCommandPending(true);
    try {
      const view = await verifyFallbackModel();
      applyFallbackModelView(view);
      toast.success("Local fallback verified");
    } catch (error) {
      toast.error(`Verify failed: ${String(error)}`);
      await refresh();
    } finally {
      setFallbackCommandPending(false);
    }
  }, [applyFallbackModelView, fallbackModelBusy, refresh]);

  const openFallbackFolder = useCallback(async () => {
    if (!isTauri()) return;

    try {
      await openFallbackModelFolder();
    } catch (error) {
      toast.error(`Open failed: ${String(error)}`);
    }
  }, []);

  const skipSetup = useCallback(() => {
    localStorage.setItem(setupSkipKey, "true");
  }, []);

  return {
    auth,
    busy: fallbackModelBusy || compute.computeTargetPending,
    compute: {
      targets: compute.localComputeTargets,
      updateTarget: compute.updateLocalComputeTarget,
    },
    fallback: {
      actionPending: fallbackCommandPending,
      cancelInstall: cancelFallbackInstall,
      install: installFallback,
      model: fallbackModel,
      openFolder: openFallbackFolder,
      remove: removeFallback,
      setEnabled: setFallbackEnabledSetting,
      verify: verifyFallback,
    },
    live: {
      busy: live.liveBusy,
      clearPasteShortcut: live.clearLivePasteShortcut,
      clearShortcut: live.clearLiveShortcut,
      inputDevices: live.liveInputDevices,
      preflightInput: live.preflightLiveInput,
      resetHotkey: live.resetLiveHotkey,
      settingsError: live.liveSettingsError,
      start: live.startLive,
      stop: live.stopLive,
      updateCaptureMode: live.updateLiveCaptureMode,
      updateHotkey: live.updateLiveHotkey,
      updateInputDevice: live.updateInputDevice,
      updateOverlay: live.updateLiveOverlay,
      updatePasteHotkey: live.updateLivePasteHotkey,
      view: live.liveView,
    },
    refresh,
    serverLabel,
    setupPromptRequest,
    skipSetup,
  };
}
