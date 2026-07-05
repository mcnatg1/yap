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
  readTranscriptHistory,
  recordTranscriptHistory,
  removeTranscriptHistory,
  writeTranscriptHistory,
  type TranscriptHistoryEntry,
} from "@/history";
import {
  acceptedFormats,
  audioExtensions,
  audioExts,
  basename,
  createInitialPipelineState,
  deriveSetupState,
  extension,
  isRecordingActive,
  isRecordingFinished,
  isRecordingRetryable,
  isRecordingRunnable,
  recordingStatusForStartFailure,
  serverConnectionLabel,
  setupStateLabel,
  type LiveCaptureMode,
  type LiveInputDeviceView,
  type LiveSessionView,
  type RailAction,
  type RecordingJobView,
  type ServerConnectionState,
  type SetupState,
  type WorkspaceView,
  workspaceCopy,
} from "@/lib/app-types";
import { historyEntryToRecordingJob } from "@/lib/history-utils";
import { cn } from "@/lib/utils";
import {
  clearLiveHotkey,
  listInputDevices,
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
} from "@/live";
import {
  listenTranscribeEvents,
  SttInvokeError,
  startTranscribe,
  transcriptFileError,
  type TranscribeBatchCompleteEvent,
  type TranscribeFileCompleteEvent,
  type TranscribeProgressEvent,
} from "@/stt";

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
  const [model, setModel] = useState("Moonshine tiny");
  const [auth, setAuth] = useState("Checking");
  const [engineBinaryStatus, setEngineBinaryStatus] = useState("Checking");
  const [engineReady, setEngineReady] = useState(false);
  const [fallbackEnabled, setFallbackEnabled] = useState(true);
  const [modelInstalled, setModelInstalled] = useState(false);
  const [setupState, setSetupState] = useState<SetupState>("checking");
  const [serverState] = useState<ServerConnectionState>("not_set");
  const [setupRoot, setSetupRoot] = useState("");
  const [setupBusy, setSetupBusy] = useState(false);
  const [liveView, setLiveView] = useState<LiveSessionView>(initialLiveView);
  const [liveInputDevices, setLiveInputDevices] = useState<LiveInputDeviceView[]>([]);
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
  const pathToItemId = useRef<Map<string, number>>(new Map());
  const setupPrompted = useRef(false);

  const hasRunnable = useMemo(
    () => queue.some((item) => isRecordingRunnable(item.status)),
    [queue],
  );
  const completed = queue.filter((item) => isRecordingFinished(item.status)).length;
  const queueProgress = queue.length ? Math.round((completed / queue.length) * 100) : 0;
  const runningItem = queue.find((item) => isRecordingActive(item.status));
  const elapsedSeconds = useElapsedSeconds(runningSince);
  const serverLabel = serverConnectionLabel(serverState);
  const setupLabel = setupStateLabel(setupState);
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

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let unlistenLive: (() => void) | undefined;
    let unlistenLiveSettings: (() => void) | undefined;

    void listenTranscribeEvents({
      onProgress: (event) => {
        updateItemProgress(event);
      },
      onFileComplete: (event) => {
        applyFileResult(event);
      },
      onComplete: (event) => {
        finishBatch(event);
      },
      onError: (error) => {
        const message = error.message || "Transcription failed";
        const pendingIds = new Set(pathToItemId.current.values());
        setQueue((items) =>
          items.map((entry) =>
            pendingIds.has(entry.id) && !isRecordingFinished(entry.status)
              ? {
                  ...entry,
                  error: message,
                  pipeline: {
                    ...entry.pipeline,
                    transcription: "error",
                  },
                  progressMessage: undefined,
                  progressPercent: undefined,
                  progressPhase: undefined,
                  status: "failed",
                }
              : entry,
          ),
        );
        setRunning(false);
        setRunningSince(undefined);
        pathToItemId.current.clear();
        setStatus("Needs attention");
        setAuth("Check local engine");
        toast.error(message);
      },
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
    });

    void listenLiveSession(setLiveView).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLive = stop;
    });

    void listen("open-live-settings", () => {
      setActiveRail("details");
      setDetailsOpen(true);
      void loadStatus();
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLiveSettings = stop;
    });

    return () => {
      cancelled = true;
      unlisten?.();
      unlistenLive?.();
      unlistenLiveSettings?.();
    };
  }, []);

  function updateQueueItem(path: string, updater: (item: RecordingJobView) => RecordingJobView) {
    setQueue((items) =>
      items.map((entry) => (entry.path === path ? updater(entry) : entry)),
    );
  }

  function updateItemProgress(event: TranscribeProgressEvent) {
    updateQueueItem(event.path, (entry) => ({
      ...entry,
      error: undefined,
      pipeline: {
        ...entry.pipeline,
        intake: "done",
        transcription: "running",
      },
      progressPhase: event.phase,
      progressPercent: event.percent,
      progressMessage: event.message,
      route: "localFallback",
      status: event.phase === "writing" ? "saving" : "local_transcribing",
    }));
    setStatus(event.message);
    const itemId = pathToItemId.current.get(event.path);
    if (itemId !== undefined) setSelectedId(itemId);
  }

  function applyFileResult(event: TranscribeFileCompleteEvent) {
    const { result } = event;
    const itemId = pathToItemId.current.get(event.path);

    if (result.error) {
      const message = transcriptFileError(result) ?? "Transcription failed.";
      updateQueueItem(event.path, (entry) => ({
        ...entry,
        error: message,
        pipeline: {
          ...entry.pipeline,
          transcription: "error",
        },
        progressPhase: undefined,
        progressPercent: undefined,
        progressMessage: undefined,
        status: "failed",
      }));
      toast.error(`${basename(event.path)}: ${message}`);
      return;
    }

    updateQueueItem(event.path, (entry) => ({
      ...entry,
      output: result.output,
      error: undefined,
      pipeline: {
        ...entry.pipeline,
        intake: "done",
        transcription: "done",
        postprocessing: "done",
      },
      progressPhase: "done",
      progressPercent: 100,
      progressMessage: "Transcript saved",
      status: "complete",
    }));
    recordHistoryEntries([
      {
        createdAt: new Date().toISOString(),
        name: basename(event.path),
        outputPath: result.output,
        sourcePath: event.path,
      },
    ]);
    void loadTranscriptText(result.output).catch(() => undefined);
    if (itemId !== undefined) setSelectedId(itemId);
  }

  function finishBatch(event: TranscribeBatchCompleteEvent) {
    setStatus(event.failed ? "Needs attention" : "Ready");
    setAuth("Ready");
    if (event.succeeded) {
      toast.success(`Transcribed ${event.succeeded} file${event.succeeded === 1 ? "" : "s"}`);
    }
    setRunning(false);
    setRunningSince(undefined);
    pathToItemId.current.clear();
  }

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
      const setup = await invoke<SetupStatus>("setup_status");
      applySetupStatus(setup);
      await loadLiveControls();
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

  function applySetupStatus(setup: SetupStatus) {
    const nextSetupState = deriveSetupState({
      engineReady: setup.engineReady,
      fallbackEnabled: setup.fallbackEnabled,
      modelInstalled: setup.modelInstalled,
    });

    setModel(setup.model.replace("cstr/", "").replace(".gguf", ""));
    setStatus(setup.engineReady ? setup.engineStatus : "Setup");
    setAuth(setup.engineReady ? "Ready" : "Setup");
    setEngineBinaryStatus(setup.engineBinaryStatus);
    setEngineReady(setup.engineReady);
    setFallbackEnabled(setup.fallbackEnabled);
    setModelInstalled(setup.modelInstalled);
    setSetupRoot(setup.root);
    setSetupState(nextSetupState);

    if (nextSetupState === "fallback_ready") {
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

    if (!setup.engineReady && !setupPrompted.current && localStorage.getItem(setupSkipKey) !== "true") {
      setupPrompted.current = true;
      setActiveRail("details");
      setDetailsOpen(true);
    }
  }

  async function installFallback() {
    if (!isTauri() || setupBusy) return;

    setSetupBusy(true);
    setSetupState("fallback_installing");
    setStatus("Installing local fallback");
    try {
      const setup = await invoke<SetupStatus>("install_local_fallback");
      localStorage.removeItem(setupSkipKey);
      applySetupStatus(setup);
      toast.success("Local fallback installed");
    } catch (error) {
      setSetupState("setup_error");
      toast.error(`Install failed: ${String(error)}`);
      await loadStatus();
    } finally {
      setSetupBusy(false);
    }
  }

  async function removeFallback() {
    if (!isTauri() || setupBusy) return;

    setSetupBusy(true);
    try {
      const setup = await invoke<SetupStatus>("remove_local_fallback");
      applySetupStatus(setup);
      toast.success("Local fallback files removed");
    } catch (error) {
      setSetupState("setup_error");
      toast.error(`Remove failed: ${String(error)}`);
      await loadStatus();
    } finally {
      setSetupBusy(false);
    }
  }

  async function setFallbackEnabledSetting(enabled: boolean) {
    if (!isTauri() || setupBusy) return;

    setSetupBusy(true);
    try {
      const setup = await invoke<SetupStatus>("set_local_fallback_enabled", { enabled });
      if (!enabled) localStorage.setItem(setupSkipKey, "true");
      applySetupStatus(setup);
      toast.success(enabled ? "Local fallback enabled" : "Local fallback disabled");
    } catch (error) {
      setSetupState("setup_error");
      toast.error(`Update failed: ${String(error)}`);
      await loadStatus();
    } finally {
      setSetupBusy(false);
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

    pathToItemId.current = new Map(pending.map((item) => [item.path, item.id]));
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
      pathToItemId.current.clear();
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

  function recordHistoryEntries(entries: TranscriptHistoryEntry[]) {
    if (!entries.length) return;

    setHistory((current) => {
      const next = entries.reduce(recordTranscriptHistory, current);
      try {
        writeTranscriptHistory(next);
      } catch (error) {
        console.warn("Transcript history could not be saved.", error);
      }
      return next;
    });
  }

  function removeHistoryEntry(outputPath: string) {
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
    toast.success("Removed from history");
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
          onLoadPreviewText={(entry) => loadTranscriptText(entry.outputPath)}
          onOpen={(entry) => void openAppPath(entry.outputPath)}
          onOpenHelp={() => handleRailAction("help")}
          onPreview={(entry) => void previewHistoryEntry(entry)}
          onRemove={removeHistoryEntry}
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
        engineReady={engineReady}
        engineBinaryStatus={engineBinaryStatus}
        fallbackEnabled={fallbackEnabled}
        model={model}
        modelInstalled={modelInstalled}
        liveBusy={liveBusy}
        liveInputDevices={liveInputDevices}
        liveSettingsError={liveSettingsError}
        liveView={liveView}
        onClearLiveHotkey={clearLiveShortcut}
        onInstallFallback={() => void installFallback()}
        onPreflightLiveInput={preflightLiveInput}
        onResetLiveHotkey={resetLiveHotkey}
        onOpenChange={(open) => {
          setDetailsOpen(open);
          if (!open && activeRail === "details") setActiveRail(workspaceView);
        }}
        onRemoveFallback={() => void removeFallback()}
        onSetInputDevice={updateInputDevice}
        onSetFallbackEnabled={(enabled) => void setFallbackEnabledSetting(enabled)}
        onSetLiveCaptureMode={updateLiveCaptureMode}
        onSetLiveHotkey={updateLiveHotkey}
        onSetLiveOverlayEnabled={updateLiveOverlay}
        onSkipSetup={skipSetup}
        onStartLive={startLive}
        onStopLive={stopLive}
        open={detailsOpen}
        serverLabel={serverLabel}
        setupLabel={setupLabel}
        setupRoot={setupRoot}
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
