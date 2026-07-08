import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { AppChrome } from "@/components/app/app-chrome";
import { AppSidebar } from "@/components/app/app-sidebar";
import { HelpSheet, SettingsSheet } from "@/components/panels/app-sheets";
import { DropHero } from "@/components/panels/drop-hero";
import { HistoryPanel } from "@/components/panels/history-panel";
import { PolishPanel } from "@/components/panels/polish-panel";
import { QueuePanel } from "@/components/panels/queue-panel";
import { TranscriptPanel } from "@/components/panels/transcript-panel";
import { WorkspaceHeader } from "@/components/panels/workspace-header";
import { TranscriptPreviewDialog } from "@/components/transcript-preview-dialog";
import { TranscriptReviewDialog } from "@/components/transcript-review-dialog";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { useElapsedSeconds } from "@/hooks/use-elapsed-seconds";
import {
  hideTranscriptHistory,
  readHiddenTranscriptHistory,
  readTranscriptHistory,
  recordTranscriptHistory,
  removeTranscriptHistory,
  writeHiddenTranscriptHistory,
  writeTranscriptHistory,
  type TranscriptHistoryEntry,
} from "@/history";
import {
  acceptedFormats,
  audioExtensions,
  audioExts,
  basename,
  createInitialPipelineState,
  deriveSetupStateFromFallbackModel,
  extension,
  isFallbackModelBusy,
  isRecordingActive,
  isRecordingFinished,
  isRecordingRetryable,
  isRecordingRunnable,
  isWorkspaceView,
  recordingStatusForStartFailure,
  serverConnectionLabel,
  type FallbackModelView,
  type LocalComputeTargetView,
  type LiveCaptureMode,
  type LiveInputDeviceView,
  type LiveSessionView,
  type RailAction,
  type RecordingJobView,
  type ServerConnectionState,
  type WorkspaceView,
  workspaceCopy,
} from "@/lib/app-types";
import { historyEntryToRecordingJob } from "@/lib/history-utils";
import { cn } from "@/lib/utils";
import {
  clearLiveHotkey,
  listInputDevices,
  listSavedLiveSessions,
  listenLiveSession,
  liveStatus,
  preflightInputDevice,
  setInputDevice,
  setLiveCaptureMode,
  setLiveHotkey,
  setLiveOverlayEnabled,
  showLiveOverlay,
  startLiveSession,
  stopLiveSession,
  type SavedLiveSession,
} from "@/live";
import {
  cancelFallbackModelInstall,
  fallbackModelStatus,
  installFallbackModel,
  listLocalComputeTargets,
  listenFallbackModelProgress,
  listenFallbackModelStatus,
  openFallbackModelFolder,
  removeFallbackModel,
  setFallbackModelEnabled,
  setLocalComputeTarget,
  verifyFallbackModel,
} from "@/settings";
import { SttInvokeError, startTranscribe } from "@/stt";

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
const defaultLiveHotkey = "Ctrl+Shift+Space";
const batchServerUnavailableMessage = "Server batch transcription is not wired yet.";

const initialLiveView: LiveSessionView = {
  captureMode: "pushToTalk",
  hotkey: defaultLiveHotkey,
  route: "none",
  status: "idle",
  visibility: "enabled",
};

function savedLiveSessionToHistoryEntry(session: SavedLiveSession): TranscriptHistoryEntry {
  const createdAt = Number.isFinite(session.createdAtMs) && session.createdAtMs > 0
    ? new Date(session.createdAtMs).toISOString()
    : new Date().toISOString();

  return {
    name: session.name,
    sourcePath: session.sourcePath,
    outputPath: session.outputPath,
    createdAt,
  };
}

type ReviewMorphOrigin = {
  height: number;
  left: number;
  top: number;
  width: number;
};

export default function App() {
  const [queue, setQueue] = useState<RecordingJobView[]>([]);
  const [nextId, setNextId] = useState(1);
  const [dragging, setDragging] = useState(false);
  const [running, setRunning] = useState(false);
  const [runningSince, setRunningSince] = useState<number>();
  const [status, setStatus] = useState("Starting");
  const [auth, setAuth] = useState("Checking");
  const [, setEngineReady] = useState(false);
  const [fallbackEnabled, setFallbackEnabled] = useState(true);
  const [fallbackModel, setFallbackModel] = useState<FallbackModelView | null>(null);
  const [modelInstalled, setModelInstalled] = useState(false);
  const [serverState] = useState<ServerConnectionState>("not_set");
  const [fallbackCommandPending, setFallbackCommandPending] = useState(false);
  const [computeTargetPending, setComputeTargetPending] = useState(false);
  const [liveView, setLiveView] = useState<LiveSessionView>(initialLiveView);
  const [liveInputDevices, setLiveInputDevices] = useState<LiveInputDeviceView[]>([]);
  const [localComputeTargets, setLocalComputeTargets] = useState<LocalComputeTargetView[]>([
    { id: "auto", label: "Auto", selected: true },
    { id: "cpu", label: "CPU", selected: false },
  ]);
  const [liveBusy, setLiveBusy] = useState(false);
  const [liveSettingsError, setLiveSettingsError] = useState("");
  const [selectedId, setSelectedId] = useState<number>();
  const [activeRail, setActiveRail] = useState<RailAction>("home");
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>("home");
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [transcriptText, setTranscriptText] = useState<Record<string, string>>({});
  const [polishedText, setPolishedText] = useState<Record<string, string>>({});
  const [history, setHistory] = useState<TranscriptHistoryEntry[]>(() => readTranscriptHistory());
  const [selectedHistoryOutput, setSelectedHistoryOutput] = useState<string>();
  const [reviewMorphOrigin, setReviewMorphOrigin] = useState<ReviewMorphOrigin>();
  const [previewEntry, setPreviewEntry] = useState<TranscriptHistoryEntry>();
  const [previewText, setPreviewText] = useState("");
  const setupPrompted = useRef(false);
  const fallbackEnabledRef = useRef(fallbackEnabled);
  const modelInstalledRef = useRef(modelInstalled);

  const hasRunnable = useMemo(
    () => queue.some((item) => isRecordingRunnable(item.status)),
    [queue],
  );
  const completed = queue.filter((item) => isRecordingFinished(item.status)).length;
  const queueProgress = queue.length ? Math.round((completed / queue.length) * 100) : 0;
  const runningItem = queue.find((item) => isRecordingActive(item.status));
  const elapsedSeconds = useElapsedSeconds(runningSince);
  const serverLabel = serverConnectionLabel(serverState);
  const selectedHistoryEntry = history.find((entry) => entry.outputPath === selectedHistoryOutput);
  const selectedHistoryItem = selectedHistoryEntry ? historyEntryToRecordingJob(selectedHistoryEntry) : undefined;
  const selectedItem =
    queue.find((item) => item.id === selectedId) ??
    selectedHistoryItem ??
    [...queue].reverse().find((item) => isRecordingFinished(item.status)) ??
    (history[0] ? historyEntryToRecordingJob(history[0]) : undefined) ??
    queue[0];
  const workspace = workspaceCopy[workspaceView];
  const showQueue = workspaceView === "transcribe";
  const showHistory = workspaceView === "home";
  const showTranscript = workspaceView === "transcribe" || workspaceView === "polish";
  const showPolish = workspaceView === "polish";
  const fallbackModelBusy = isFallbackModelBusy(fallbackModel, fallbackCommandPending);
  const setupBusy = fallbackModelBusy || computeTargetPending;

  useEffect(() => {
    fallbackEnabledRef.current = fallbackEnabled;
  }, [fallbackEnabled]);

  useEffect(() => {
    modelInstalledRef.current = modelInstalled;
  }, [modelInstalled]);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlistenLive: (() => void) | undefined;
    let unlistenLiveSaved: (() => void) | undefined;
    let unlistenFallbackProgress: (() => void) | undefined;
    let unlistenFallbackStatus: (() => void) | undefined;

    void listenLiveSession(setLiveView).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLive = stop;
    });

    void listen<SavedLiveSession>("live-session-saved", (event) => {
      const entry = savedLiveSessionToHistoryEntry(event.payload);
      recordVisibleHistoryEntries([entry], "Transcript history could not be saved.");
      setSelectedHistoryOutput(entry.outputPath);
      setActiveRail("home");
      setWorkspaceView("home");
      setStatus("Ready");
      void loadTranscriptText(entry.outputPath).catch(() => undefined);
      toast.success("Live transcript saved");
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLiveSaved = stop;
    });

    void listenFallbackModelProgress((view) => {
      applyFallbackModelView(view);
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenFallbackProgress = stop;
    });

    void listenFallbackModelStatus((view) => {
      applyFallbackModelView(view);
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenFallbackStatus = stop;
    });

    void listSavedLiveSessions()
      .then((sessions) => {
        if (cancelled) return;
        recordVisibleHistoryEntries(
          sessions.map(savedLiveSessionToHistoryEntry),
          "Live transcript history could not be synced.",
        );
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlistenLive?.();
      unlistenLiveSaved?.();
      unlistenFallbackProgress?.();
      unlistenFallbackStatus?.();
    };
  }, []);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<unknown>("open-workspace", (event) => {
      if (!isWorkspaceView(event.payload)) return;
      const action = event.payload;
      setActiveRail(action);
      setWorkspaceView(action);
      if (action === "polish") {
        setStatus(isRecordingFinished(selectedItem?.status) ? "Transcript ready" : "Transcribe a file first");
      }
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [selectedItem?.status]);

  useEffect(() => {
    void loadStatus();

    if (!isTauri()) {
      setStatus("Preview");
      setAuth("Tauri bridge");
      return;
    }

    const unlistenDrag = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "enter") setDragging(true);
      if (event.payload.type === "leave" || event.payload.type === "drop") setDragging(false);
      if (event.payload.type === "drop") addPaths(event.payload.paths);
    });

    return () => {
      void unlistenDrag.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (selectedHistoryOutput) return;

    if (!queue.length) {
      setSelectedId(undefined);
      return;
    }

    if (!selectedId || !queue.some((item) => item.id === selectedId)) {
      setSelectedId(queue[queue.length - 1].id);
    }
  }, [queue, selectedId, selectedHistoryOutput]);

  useEffect(() => {
    if (selectedHistoryOutput && !history.some((entry) => entry.outputPath === selectedHistoryOutput)) {
      setSelectedHistoryOutput(undefined);
    }
  }, [history, selectedHistoryOutput]);

  useEffect(() => {
    if (selectedItem?.output && !transcriptText[selectedItem.output]) {
      void loadTranscriptText(selectedItem.output).catch(() => toast.error("Preview unavailable"));
    }
  }, [selectedItem?.output, transcriptText]);

  async function loadStatus() {
    if (!isTauri()) return;

    try {
      const [setup, view] = await Promise.all([
        invoke<SetupStatus>("setup_status"),
        fallbackModelStatus(),
      ]);
      applySetupStatus(setup);
      applyFallbackModelView(view, {
        authText: setup.engineReady ? "Ready" : "Setup",
        engineReady: setup.engineReady,
        fallbackEnabled: setup.fallbackEnabled,
        modelInstalled: setup.modelInstalled,
      });
      await Promise.all([loadLiveControls(), loadComputeTargets()]);
    } catch (error) {
      setStatus("Setup check failed");
      setAuth(String(error));
    }
  }

  async function loadLiveControls() {
    const [live, devices] = await Promise.all([liveStatus(), listInputDevices()]);
    setLiveView(live);
    setLiveInputDevices(devices);
  }

  async function loadComputeTargets() {
    setLocalComputeTargets(await listLocalComputeTargets());
  }

  function unblockFallbackReadyQueue() {
    setQueue((items) =>
      items.map((item) =>
        item.status === "blocked_setup_required"
          ? {
              ...item,
              error: undefined,
              pipeline: {
                ...item.pipeline,
                transcription: "notStarted",
              },
              status: "queued_local_fallback",
            }
          : item,
      ),
    );
  }

  function fallbackStatusText(view: FallbackModelView, enabled: boolean) {
    switch (view.status) {
      case "downloading":
        return view.message ?? "Installing local fallback";
      case "verifying":
        return view.message ?? "Verifying local fallback";
      case "ready":
        return "Transcription engine ready";
      case "disabled":
        return "Local fallback disabled";
      case "error":
        return view.message ?? "Local fallback needs attention";
      case "missing":
      case "corrupted":
        return enabled ? "Local fallback model missing" : "Local fallback disabled";
    }
  }

  function maybeOpenSetupPrompt(nextSetupState: ReturnType<typeof deriveSetupStateFromFallbackModel>, nextFallbackEnabled: boolean) {
    if (
      !nextFallbackEnabled ||
      nextSetupState === "fallback_ready" ||
      setupPrompted.current ||
      localStorage.getItem(setupSkipKey) === "true"
    ) {
      return;
    }

    setupPrompted.current = true;
    setActiveRail("details");
    setDetailsOpen(true);
  }

  function applyFallbackModelView(
    view: FallbackModelView,
    overrides: {
      authText?: string;
      engineReady?: boolean;
      fallbackEnabled?: boolean;
      modelInstalled?: boolean;
      statusText?: string;
    } = {},
  ) {
    const nextFallbackEnabled = overrides.fallbackEnabled
      ?? (view.status === "ready" ? true : view.status === "disabled" ? false : fallbackEnabledRef.current);
    const nextModelInstalled = overrides.modelInstalled
      ?? (
        view.status === "ready" || view.status === "disabled" || view.status === "corrupted"
          ? true
          : view.status === "missing"
            ? false
            : modelInstalledRef.current
      );
    const nextEngineReady = overrides.engineReady ?? (view.status === "ready");
    const nextSetupState = deriveSetupStateFromFallbackModel(view.status, nextFallbackEnabled);

    fallbackEnabledRef.current = nextFallbackEnabled;
    modelInstalledRef.current = nextModelInstalled;
    setFallbackModel(view);
    setStatus(overrides.statusText ?? fallbackStatusText(view, nextFallbackEnabled));
    setAuth(overrides.authText ?? (nextEngineReady ? "Ready" : "Setup"));
    setEngineReady(nextEngineReady);
    setFallbackEnabled(nextFallbackEnabled);
    setModelInstalled(nextModelInstalled);
    maybeOpenSetupPrompt(nextSetupState, nextFallbackEnabled);

    if (nextSetupState === "fallback_ready") {
      unblockFallbackReadyQueue();
    }
  }

  function applySetupStatus(setup: SetupStatus) {
    fallbackEnabledRef.current = setup.fallbackEnabled;
    modelInstalledRef.current = setup.modelInstalled;
    setStatus(setup.engineReady ? setup.engineStatus : "Setup");
    setAuth(setup.engineReady ? "Ready" : "Setup");
    setEngineReady(setup.engineReady);
    setFallbackEnabled(setup.fallbackEnabled);
    setModelInstalled(setup.modelInstalled);
  }

  async function installFallback() {
    if (!isTauri() || fallbackModelBusy) return;

    setFallbackCommandPending(true);
    fallbackEnabledRef.current = true;
    setFallbackEnabled(true);
    setStatus("Installing local fallback");
    try {
      const view = await installFallbackModel();
      localStorage.removeItem(setupSkipKey);
      applyFallbackModelView(view, { fallbackEnabled: true });
      toast.success("Local fallback installed");
    } catch (error) {
      toast.error(`Install failed: ${String(error)}`);
      await loadStatus();
    } finally {
      setFallbackCommandPending(false);
    }
  }

  async function removeFallback() {
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
      await loadStatus();
    } finally {
      setFallbackCommandPending(false);
    }
  }

  async function setFallbackEnabledSetting(enabled: boolean) {
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
      await loadStatus();
    } finally {
      setFallbackCommandPending(false);
    }
  }

  async function cancelFallbackInstall() {
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
      await loadStatus();
    } finally {
      setFallbackCommandPending(false);
    }
  }

  async function verifyFallback() {
    if (!isTauri() || fallbackModelBusy) return;

    setFallbackCommandPending(true);
    try {
      const view = await verifyFallbackModel();
      applyFallbackModelView(view);
      toast.success("Local fallback verified");
    } catch (error) {
      toast.error(`Verify failed: ${String(error)}`);
      await loadStatus();
    } finally {
      setFallbackCommandPending(false);
    }
  }

  async function openFallbackFolder() {
    if (!isTauri()) return;

    try {
      await openFallbackModelFolder();
    } catch (error) {
      toast.error(`Open failed: ${String(error)}`);
    }
  }

  async function updateLocalComputeTarget(targetId: string) {
    if (!isTauri() || setupBusy) return;

    setComputeTargetPending(true);
    try {
      setLocalComputeTargets(await setLocalComputeTarget(targetId));
      toast.success("Local compute updated");
    } catch (error) {
      toast.error(String(error));
      await loadComputeTargets();
    } finally {
      setComputeTargetPending(false);
    }
  }

  function skipSetup() {
    localStorage.setItem(setupSkipKey, "true");
    setDetailsOpen(false);
    if (activeRail === "details") setActiveRail(workspaceView);
  }

  async function updateLive(action: () => Promise<LiveSessionView>, message?: string) {
    if (!isTauri() || liveBusy) return;

    setLiveBusy(true);
    try {
      setLiveSettingsError("");
      const view = await action();
      setLiveView(view);
      setLiveInputDevices(await listInputDevices());
      if (message) toast.success(message);
    } catch (error) {
      const message = String(error);
      setLiveSettingsError(message);
      toast.error(message);
    } finally {
      setLiveBusy(false);
    }
  }

  function updateLiveOverlay(enabled: boolean) {
    void updateLive(() => setLiveOverlayEnabled(enabled), enabled ? "Live overlay enabled" : "Live overlay hidden");
  }

  function updateLiveHotkey(hotkey: string) {
    const next = hotkey.trim();
    void updateLive(next ? () => setLiveHotkey(next) : clearLiveHotkey, next ? "Live shortcut updated" : "Live shortcut cleared");
  }

  function resetLiveHotkey() {
    void updateLive(() => setLiveHotkey(defaultLiveHotkey), "Live shortcut reset");
  }

  function clearLiveShortcut() {
    void updateLive(clearLiveHotkey, "Live shortcut cleared");
  }

  function updateLiveCaptureMode(captureMode: LiveCaptureMode) {
    void updateLive(() => setLiveCaptureMode(captureMode));
  }

  function updateInputDevice(deviceId?: string) {
    void updateLive(() => setInputDevice(deviceId));
  }

  function preflightLiveInput() {
    void updateLive(preflightInputDevice);
  }

  function startLive() {
    void updateLive(async () => {
      await showLiveOverlay();
      return startLiveSession();
    });
  }

  function stopLive() {
    void updateLive(stopLiveSession);
  }

  function addPaths(paths: string[]) {
    setQueue((current) => {
      const existing = new Set(current.map((item) => item.path));
      const accepted = paths.filter((path) => audioExts.has(extension(path)) && !existing.has(path));
      if (paths.length && !accepted.length) {
        toast.warning(`Drop ${acceptedFormats} files.`);
        return current;
      }

      const newItems: RecordingJobView[] = accepted.map((path, index) => ({
        error: batchServerUnavailableMessage,
        id: nextId + index,
        intent: "recording",
        name: basename(path),
        path,
        pipeline: createInitialPipelineState(),
        route: "serverBatch",
        status: "blocked_server_unavailable",
      }));
      setNextId((id) => id + newItems.length);
      if (newItems.length) {
        setActiveRail("transcribe");
        setWorkspaceView("transcribe");
        setSelectedHistoryOutput(undefined);
        setSelectedId(newItems[newItems.length - 1].id);
      }
      return [...current, ...newItems];
    });
  }

  function goToTranscribe() {
    setActiveRail("transcribe");
    setWorkspaceView("transcribe");
  }

  async function pickFiles() {
    goToTranscribe();

    if (!isTauri()) {
      toast.info("Preview only");
      return;
    }

    try {
      const selected = await openDialog({
        multiple: true,
        title: "Choose recordings",
        filters: [{ name: "Audio and video", extensions: audioExtensions }],
      });
      if (Array.isArray(selected)) addPaths(selected);
      else if (selected) addPaths([selected]);
    } catch (error) {
      toast.error(`Picker failed: ${String(error)}`);
    }
  }

  function handleRailAction(action: RailAction) {
    setActiveRail(action);

    if (action === "details") {
      setDetailsOpen(true);
      void loadStatus();
      return;
    }
    if (action === "help") {
      setHelpOpen(true);
      return;
    }

    setWorkspaceView(action);

    if (action === "polish") {
      setStatus(isRecordingFinished(selectedItem?.status) ? "Transcript ready" : "Transcribe a file first");
    }
  }

  async function transcribeItems(pending: RecordingJobView[]) {
    if (!pending.length || running || !isTauri()) return;
    if (pending.some((item) => item.intent === "recording")) {
      toast.error(batchServerUnavailableMessage);
      return;
    }

    setRunning(true);
    setRunningSince(Date.now());
    setStatus(`Transcribing 0/${pending.length}`);

    for (const [index, item] of pending.entries()) {
      if (index === 0) setSelectedId(item.id);
      setQueue((items) =>
        items.map((entry) =>
          entry.id === item.id
            ? {
                ...entry,
                error: undefined,
                pipeline: {
                  ...entry.pipeline,
                  intake: "done",
                  transcription: index === 0 ? "running" : "queued",
                },
                progressPhase: index === 0 ? "starting" : undefined,
                progressPercent: index === 0 ? 0 : undefined,
                progressMessage: index === 0 ? "Preparing..." : undefined,
                route: "localFallback",
                status: index === 0 ? "local_transcribing" : "queued_local_fallback",
              }
            : entry,
        ),
      );
    }

    try {
      await startTranscribe(pending.map((item) => item.path));
    } catch (error) {
      const failure = error instanceof SttInvokeError ? error : undefined;
      const message = failure?.message ?? String(error || "Transcription failed");
      const status = recordingStatusForStartFailure(failure?.code);
      const pendingIds = new Set(pending.map((entry) => entry.id));
      setQueue((items) =>
        items.map((entry) =>
          pendingIds.has(entry.id)
            ? {
                ...entry,
                error: message,
                pipeline: {
                  ...entry.pipeline,
                  transcription: status === "failed" ? "error" : "notStarted",
                },
                progressMessage: undefined,
                progressPercent: undefined,
                progressPhase: undefined,
                status,
              }
            : entry,
        ),
      );
      setRunning(false);
      setRunningSince(undefined);
      setStatus("Needs attention");
      toast.error(message);
    }
  }

  async function runQueue() {
    const pending = queue.filter((item) => isRecordingRunnable(item.status));
    await transcribeItems(pending);
  }

  async function retryItem(id: number) {
    const item = queue.find((entry) => entry.id === id);
    if (!item || !isRecordingRetryable(item.status) || running) return;

    setSelectedId(id);
    await transcribeItems([{ ...item, status: "queued_local_fallback", error: undefined }]);
  }

  function removeItem(id: number) {
    setQueue((items) => {
      const item = items.find((entry) => entry.id === id);
      if (!item || isRecordingActive(item.status)) return items;
      if (item.status === "cancelled") return items.filter((entry) => entry.id !== id);

      return items.map((entry) =>
        entry.id === id
          ? {
              ...entry,
              error: undefined,
              pipeline: {
                ...entry.pipeline,
                transcription: entry.pipeline.transcription === "running" ? "skipped" : entry.pipeline.transcription,
              },
              progressMessage: undefined,
              progressPercent: undefined,
              progressPhase: undefined,
              status: "cancelled",
            }
          : entry,
      );
    });
  }

  function selectQueueItem(id: number) {
    setSelectedHistoryOutput(undefined);
    setSelectedId(id);
  }

  function clearQueue() {
    if (!running) {
      setQueue([]);
      setTranscriptText({});
    }
  }

  async function loadTranscriptText(path: string) {
    if (transcriptText[path]) return transcriptText[path];
    if (!isTauri()) return "";

    const text = await invoke<string>("read_text_file", { path });
    setTranscriptText((current) => ({ ...current, [path]: text }));
    return text;
  }

  async function copyTranscript(item: RecordingJobView) {
    if (!item.output) return;

    try {
      const text = await loadTranscriptText(item.output);
      await navigator.clipboard.writeText(text || item.output);
      toast.success(text ? "Transcript copied" : "Path copied");
    } catch {
      toast.error("Copy failed");
    }
  }

  async function openAppPath(path: string) {
    try {
      await invoke("open_app_path", { path });
      toast.success("Opened file");
    } catch {
      toast.error("Open failed");
    }
  }

  async function revealPath(path: string) {
    try {
      await invoke("reveal_app_path", { path });
    } catch {
      toast.error("Reveal failed");
    }
  }

  async function savePolishedTranscript(item: RecordingJobView, text: string) {
    if (!item.output || !text.trim()) return "";

    try {
      const path = await invoke<string>("write_polished_text", { path: item.output, text });
      toast.success("Polished draft saved");
      return path;
    } catch (error) {
      toast.error("Save failed");
      throw error;
    }
  }

  function recordVisibleHistoryEntries(entries: TranscriptHistoryEntry[], warning: string) {
    if (!entries.length) return;
    const hiddenHistoryOutputs = new Set(readHiddenTranscriptHistory());
    const visibleEntries = entries.filter((entry) => !hiddenHistoryOutputs.has(entry.outputPath));
    if (!visibleEntries.length) return;

    setHistory((current) => {
      const next = visibleEntries.reduce(recordTranscriptHistory, current);
      try {
        writeTranscriptHistory(next);
      } catch (error) {
        console.warn(warning, error);
      }
      return next;
    });
  }

  function rememberHiddenHistoryEntry(outputPath: string) {
    const next = hideTranscriptHistory(readHiddenTranscriptHistory(), outputPath);
    try {
      writeHiddenTranscriptHistory(next);
    } catch (error) {
      console.warn("Hidden transcript history could not be saved.", error);
    }
  }

  function forgetHistoryEntry(outputPath: string) {
    setHistory((current) => {
      const next = removeTranscriptHistory(current, outputPath);
      try {
        writeTranscriptHistory(next);
      } catch (error) {
        console.warn("Transcript history removal could not be saved.", error);
      }
      return next;
    });
    if (selectedHistoryOutput === outputPath) setSelectedHistoryOutput(undefined);
  }

  function hideHistoryEntry(outputPath: string) {
    rememberHiddenHistoryEntry(outputPath);
    forgetHistoryEntry(outputPath);
    toast.success("Hidden from history");
  }

  async function deleteHistoryEntry(entry: TranscriptHistoryEntry) {
    try {
      await invoke("delete_history_entry_files", {
        outputPath: entry.outputPath,
        sourcePath: entry.sourcePath,
      });
      rememberHiddenHistoryEntry(entry.outputPath);
      forgetHistoryEntry(entry.outputPath);
      setTranscriptText((current) => {
        const { [entry.outputPath]: _deleted, ...next } = current;
        return next;
      });
      toast.success("Deleted from device");
    } catch (error) {
      toast.error(String(error || "Delete failed"));
    }
  }

  function selectHistoryEntry(entry: TranscriptHistoryEntry, origin?: DOMRect) {
    setSelectedId(undefined);
    setSelectedHistoryOutput(entry.outputPath);
    setReviewMorphOrigin(
      origin
        ? {
            height: origin.height,
            left: origin.left,
            top: origin.top,
            width: origin.width,
          }
        : undefined,
    );
    setActiveRail("home");
    setWorkspaceView("home");
  }

  async function previewHistoryEntry(entry: TranscriptHistoryEntry) {
    setPreviewEntry(entry);
    setPreviewText("");

    try {
      setPreviewText(await loadTranscriptText(entry.outputPath));
    } catch {
      setPreviewText("Preview unavailable. Open the transcript file from the actions menu.");
    }
  }

  const workspaceLeftPane = (
    <>
      {showQueue ? (
        <QueuePanel
          completed={completed}
          elapsedSeconds={elapsedSeconds}
          hasRunnable={hasRunnable}
          onClear={clearQueue}
          onRemove={removeItem}
          onRetry={(id) => void retryItem(id)}
          onReveal={(path) => void revealPath(path)}
          onRun={() => void runQueue()}
          onSelect={selectQueueItem}
          queue={queue}
          queueProgress={queueProgress}
          running={running}
          runningItem={runningItem}
          selectedId={selectedId}
        />
      ) : null}

      {showHistory ? (
        <HistoryPanel
          entries={history}
          onCopy={(entry) => void copyTranscript(historyEntryToRecordingJob(entry))}
          onDelete={(entry) => void deleteHistoryEntry(entry)}
          onHide={hideHistoryEntry}
          onLoadPreviewText={(entry) => loadTranscriptText(entry.outputPath)}
          onOpen={(entry) => void openAppPath(entry.outputPath)}
          onOpenHelp={() => handleRailAction("help")}
          onPreview={(entry) => void previewHistoryEntry(entry)}
          onReveal={(entry) => void revealPath(entry.outputPath)}
          onSelect={selectHistoryEntry}
          selectedOutputPath={selectedHistoryOutput}
        />
      ) : null}

      {showPolish ? (
        <PolishPanel
          item={selectedItem}
          onLoadText={loadTranscriptText}
          onOpenHelp={() => handleRailAction("help")}
          onPolished={(outputPath, text) => {
            setPolishedText((current) => ({ ...current, [outputPath]: text }));
            toast.success("Polished draft ready");
          }}
          onSave={savePolishedTranscript}
          originalText={selectedItem?.output ? transcriptText[selectedItem.output] : undefined}
          polishedText={selectedItem?.output ? polishedText[selectedItem.output] : undefined}
        />
      ) : null}
    </>
  );
  const workspaceTranscriptPane = showTranscript ? (
    <div className="h-full min-w-0">
      <TranscriptPanel
        elapsedSeconds={elapsedSeconds}
        item={selectedItem}
        onCopy={copyTranscript}
        onOpen={(path) => void openAppPath(path)}
        onOpenHelp={() => handleRailAction("help")}
        onRetry={(id) => void retryItem(id)}
        onReveal={(path) => void revealPath(path)}
        running={running}
        text={selectedItem?.output ? transcriptText[selectedItem.output] : undefined}
      />
    </div>
  ) : null;
  const workspaceMain = (
    <div
      className={cn(
        "grid w-full min-w-0 gap-5",
        showTranscript
          ? "grid-cols-1 xl:grid-cols-[minmax(0,1fr)_minmax(380px,0.78fr)]"
          : "grid-cols-1",
      )}
    >
      {workspaceLeftPane}
      {workspaceTranscriptPane}
    </div>
  );
  const appWorkspace = (
    <section className="surface-workspace scrollbar-none h-full min-h-0 w-full min-w-0 flex-1 overflow-x-hidden overflow-y-auto bg-card p-4">
      <WorkspaceHeader
        auth={auth}
        description={workspace.description}
        historyCount={history.length}
        onOpenDetails={() => handleRailAction("details")}
        onOpenHelp={() => handleRailAction("help")}
        status={status}
        title={workspace.title}
      />

      {workspaceView === "transcribe" ? (
        <DropHero
          dragging={dragging}
          onDragLeave={() => setDragging(false)}
          onDragOver={(event) => {
            event.preventDefault();
            setDragging(true);
          }}
          onDrop={(event) => {
            event.preventDefault();
            setDragging(false);
            if (!isTauri()) toast.info("Preview only");
          }}
          onOpenHelp={() => handleRailAction("help")}
          onPickFiles={() => void pickFiles()}
        />
      ) : null}

      <section className="mt-7 w-full min-w-0">
        {workspaceMain}
      </section>
    </section>
  );

  return (
    <SidebarProvider
      className="h-screen overflow-hidden bg-background text-foreground"
      onOpenChange={(open) => setRailCollapsed(!open)}
      open={!railCollapsed}
    >
      <AppSidebar active={activeRail} onAction={handleRailAction} />
      <SidebarInset className="flex min-h-0 flex-col overflow-hidden">
        <AppChrome />
        <div className="min-h-0 w-full min-w-0 flex-1 overflow-hidden bg-background pb-4 pr-4 pt-0">
          {appWorkspace}
        </div>
      </SidebarInset>
      <SettingsSheet
        auth={auth}
        busy={setupBusy}
        fallbackActionPending={fallbackCommandPending}
        fallbackModel={fallbackModel}
        liveBusy={liveBusy}
        liveInputDevices={liveInputDevices}
        liveSettingsError={liveSettingsError}
        liveView={liveView}
        localComputeTargets={localComputeTargets}
        onCancelFallbackInstall={() => void cancelFallbackInstall()}
        onClearLiveHotkey={clearLiveShortcut}
        onInstallFallback={() => void installFallback()}
        onOpenFallbackFolder={() => void openFallbackFolder()}
        onPreflightLiveInput={preflightLiveInput}
        onResetLiveHotkey={resetLiveHotkey}
        onOpenChange={(open) => {
          setDetailsOpen(open);
          if (!open && activeRail === "details") setActiveRail(workspaceView);
        }}
        onRemoveFallback={() => void removeFallback()}
        onSetInputDevice={updateInputDevice}
        onSetFallbackEnabled={(enabled) => void setFallbackEnabledSetting(enabled)}
        onVerifyFallback={() => void verifyFallback()}
        onSetLiveCaptureMode={updateLiveCaptureMode}
        onSetLiveHotkey={updateLiveHotkey}
        onSetLiveOverlayEnabled={updateLiveOverlay}
        onSetLocalComputeTarget={(targetId) => void updateLocalComputeTarget(targetId)}
        onSkipSetup={skipSetup}
        onStartLive={startLive}
        onStopLive={stopLive}
        open={detailsOpen}
        serverLabel={serverLabel}
        status={status}
      />
      <HelpSheet
        onOpenChange={(open) => {
          setHelpOpen(open);
          if (!open && activeRail === "help") setActiveRail(workspaceView);
        }}
        open={helpOpen}
      />
      <TranscriptReviewDialog
        elapsedSeconds={elapsedSeconds}
        item={selectedHistoryItem}
        morphOrigin={reviewMorphOrigin}
        onCopy={copyTranscript}
        onOpen={(path) => void openAppPath(path)}
        onOpenChange={(open) => {
          if (!open) {
            setSelectedHistoryOutput(undefined);
            setReviewMorphOrigin(undefined);
          }
        }}
        onOpenHelp={() => handleRailAction("help")}
        onRetry={(id) => void retryItem(id)}
        onReveal={(path) => void revealPath(path)}
        open={workspaceView === "home" && Boolean(selectedHistoryItem)}
        running={running}
        text={selectedHistoryItem?.output ? transcriptText[selectedHistoryItem.output] : undefined}
      />
      <TranscriptPreviewDialog
        entry={previewEntry}
        onCopy={(entry) => void copyTranscript(historyEntryToRecordingJob(entry))}
        onOpen={(entry) => void openAppPath(entry.outputPath)}
        onOpenChange={(open) => {
          if (!open) setPreviewEntry(undefined);
        }}
        onReveal={(entry) => void revealPath(entry.outputPath)}
        text={previewText}
      />
    </SidebarProvider>
  );
}
