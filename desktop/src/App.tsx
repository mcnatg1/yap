import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { useLocalComputeTargets } from "@/hooks/use-local-compute-targets";
import { useLiveControl } from "@/hooks/use-live-control";
import { useRecordingSelection } from "@/hooks/use-recording-selection";
import { useRegisteredPlayback } from "@/hooks/use-registered-playback";
import { useRecordingDrop } from "@/hooks/use-recording-drop";
import { useServerConnection } from "@/hooks/use-server-connection";
import { useTranscriptFileActions } from "@/hooks/use-transcript-file-actions";
import { useTranscriptPreview } from "@/hooks/use-transcript-preview";
import { useTranscriptText } from "@/hooks/use-transcript-text";
import { useTranscriptHistory } from "@/hooks/use-transcript-history";
import { useWorkspaceNavigation } from "@/hooks/use-workspace-navigation";
import {
  savedSessionToTranscriptHistoryEntry,
  type TranscriptHistoryEntry,
} from "@/history";
import {
  acceptedFormats,
  acceptedRecordingDrops,
  audioExtensions,
  deriveSetupStateFromFallbackModel,
  isFallbackModelBusy,
  isRecordingActive,
  isRecordingFinished,
  isRecordingRetryable,
  isRecordingRunnable,
  recordingStatusForStartFailure,
  type FallbackModelView,
  type RecordingJobView,
  workspaceCopy,
} from "@/lib/app-types";
import {
  allowRecordingPlaybackPath,
} from "@/lib/playback-registry";
import {
  fallbackStatusText,
  shouldOpenSetupPrompt,
  unblockFallbackReadyQueue,
} from "@/lib/setup-model-state";
import { cn } from "@/lib/utils";
import {
  availableQueuedServerSlots,
  createQueuedServerRecordingJobs,
  nextRecordingQueueId,
  readRecordingQueue,
  writeRecordingQueue,
} from "@/recording-queue";
import {
  deleteRecoverableLiveSession,
  listRecoverableLiveSessions,
  listSavedLiveSessions,
  recoverLiveSession,
  type SavedLiveSession,
} from "@/live";
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
const batchServerQueuedMessage = "Queued until a transcription server is connected.";

export default function App() {
  const initialQueue = useMemo(() => readRecordingQueue(), []);
  const [queue, setQueue] = useState<RecordingJobView[]>(initialQueue);
  const nextRecordingId = useRef(nextRecordingQueueId(initialQueue));
  const [running, setRunning] = useState(false);
  const [runningSince, setRunningSince] = useState<number>();
  const [status, setStatus] = useState("Starting");
  const [auth, setAuth] = useState("Checking");
  const [, setEngineReady] = useState(false);
  const [fallbackEnabled, setFallbackEnabled] = useState(true);
  const [fallbackModel, setFallbackModel] = useState<FallbackModelView | null>(null);
  const [modelInstalled, setModelInstalled] = useState(false);
  const [fallbackCommandPending, setFallbackCommandPending] = useState(false);
  const { refreshServerState, serverLabel } = useServerConnection();
  const {
    clearLivePasteShortcut,
    clearLiveShortcut,
    liveBusy,
    liveInputDevices,
    liveSettingsError,
    liveView,
    preflightLiveInput,
    refreshLiveState,
    resetLiveHotkey,
    startLive,
    stopLive,
    updateInputDevice,
    updateLiveCaptureMode,
    updateLiveHotkey,
    updateLiveOverlay,
    updateLivePasteHotkey,
  } = useLiveControl();
  const {
    clearTranscriptText,
    forgetTranscriptText,
    loadTranscriptPreviewText,
    loadTranscriptText,
    polishedText,
    rememberPolishedText,
    transcriptText,
  } = useTranscriptText();
  const {
    copyTranscript,
    openAppPath,
    revealPath,
    savePolishedTranscript,
  } = useTranscriptFileActions(loadTranscriptText);
  const {
    forgetHistoryEntry,
    history,
    reconcileHiddenHistory,
    reconcileNativeHistoryEntries,
    recordVisibleHistoryEntries,
    rememberHiddenHistoryEntry,
  } = useTranscriptHistory();
  const historyPlaybackPaths = useRegisteredPlayback(queue, setQueue, history);
  const {
    clearHistorySelectionIf,
    closeHistoryReview,
    historyJob,
    reviewMorphOrigin,
    selectHistoryEntry,
    selectQueueItem,
    selectQueueItemOnly,
    selectedHistoryItem,
    selectedHistoryOutput,
    selectedId,
    selectedItem,
  } = useRecordingSelection({ history, historyPlaybackPaths, queue });
  const setupPrompted = useRef(false);
  const fallbackEnabledRef = useRef(fallbackEnabled);
  const modelInstalledRef = useRef(modelInstalled);
  const queueRef = useRef(queue);
  const recordingDrop = useRecordingDrop(addPaths);

  const hasRunnable = useMemo(
    () => queue.some((item) => isRecordingRunnable(item.status)),
    [queue],
  );
  const completed = queue.filter((item) => isRecordingFinished(item.status)).length;
  const queueProgress = queue.length ? Math.round((completed / queue.length) * 100) : 0;
  const runningItem = queue.find((item) => isRecordingActive(item.status));
  const elapsedSeconds = useElapsedSeconds(runningSince);
  const {
    activeRail,
    closeDetails,
    detailsOpen,
    helpOpen,
    onDetailsOpenChange,
    onHelpOpenChange,
    openWorkspace,
    railCollapsed,
    setRailCollapsed,
    showDetails,
    workspaceView,
  } = useWorkspaceNavigation({
    onOpenDetails: () => void loadStatus(),
    onOpenPolish: () => {
      setStatus(isRecordingFinished(selectedItem?.status) ? "Transcript ready" : "Transcribe a file first");
    },
  });
  const workspace = workspaceCopy[workspaceView];
  const showQueue = workspaceView === "transcribe";
  const showHistory = workspaceView === "home";
  const showTranscript = workspaceView === "transcribe" || workspaceView === "polish";
  const showPolish = workspaceView === "polish";
  const fallbackModelBusy = isFallbackModelBusy(fallbackModel, fallbackCommandPending);
  const {
    computeTargetPending,
    loadComputeTargets,
    localComputeTargets,
    updateLocalComputeTarget,
  } = useLocalComputeTargets(fallbackModelBusy);
  const setupBusy = fallbackModelBusy || computeTargetPending;

  useEffect(() => {
    fallbackEnabledRef.current = fallbackEnabled;
  }, [fallbackEnabled]);

  useEffect(() => {
    modelInstalledRef.current = modelInstalled;
  }, [modelInstalled]);

  useEffect(() => {
    queueRef.current = queue;
    try {
      writeRecordingQueue(queue);
    } catch (error) {
      console.warn("Queued recordings could not be saved.", error);
      toast.warning("Queued recordings could not be saved.");
    }
  }, [queue]);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlistenLiveSaved: (() => void) | undefined;
    let unlistenFallbackProgress: (() => void) | undefined;
    let unlistenFallbackStatus: (() => void) | undefined;

    void listen<SavedLiveSession>("live-session-saved", (event) => {
      const entry = savedSessionToTranscriptHistoryEntry(event.payload);
      const recorded = recordVisibleHistoryEntries([entry], "Transcript history could not be saved.");
      if (!recorded) return;
      selectHistoryEntry(entry);
      openWorkspace("home");
      setStatus("Ready");
      void loadTranscriptText(entry.outputPath).catch(() => undefined);
      if (entry.warning) {
        toast.warning(entry.warning);
      } else {
        toast.success("Live transcript saved");
      }
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

    void reconcileHiddenHistory()
      .then(async () => {
        const [saved, recoverable] = await Promise.all([
          listSavedLiveSessions(),
          listRecoverableLiveSessions(),
        ]);
        return [
          ...saved,
          ...recoverable.map((session): SavedLiveSession => ({
            createdAtMs: Math.max(0, session.expiresAtMs - 24 * 60 * 60 * 1000),
            name: session.name,
            outputPath: session.audioPartialPath ?? session.journalPartialPath ?? session.name,
            sourcePath: session.audioPartialPath ?? session.journalPartialPath ?? session.name,
            warning: session.reason,
            recoveryState: "recoverable",
          })),
        ];
      })
      .then((sessions) => {
        if (cancelled) return;
        reconcileNativeHistoryEntries(
          sessions.map(savedSessionToTranscriptHistoryEntry),
          "Live transcript history could not be synced.",
        );
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlistenLiveSaved?.();
      unlistenFallbackProgress?.();
      unlistenFallbackStatus?.();
    };
  }, []);

  useEffect(() => {
    void loadStatus();

    if (!isTauri()) {
      setStatus("Preview");
      setAuth("Tauri bridge");
    }
  }, []);

  useEffect(() => {
    if (selectedItem?.output && !Object.prototype.hasOwnProperty.call(transcriptText, selectedItem.output)) {
      void loadTranscriptText(selectedItem.output).catch(() => toast.error("Preview unavailable"));
    }
  }, [selectedItem?.output, transcriptText]);

  async function loadStatus() {
    if (!isTauri()) return;

    try {
      const [setup, view] = await Promise.all([
        invoke<SetupStatus>("setup_status"),
        fallbackModelStatus(),
        refreshServerState(),
      ]);
      applySetupStatus(setup);
      applyFallbackModelView(view, {
        authText: setup.engineReady ? "Ready" : "Setup",
        engineReady: setup.engineReady,
        fallbackEnabled: setup.fallbackEnabled,
        modelInstalled: setup.modelInstalled,
      });
      await Promise.all([refreshLiveState(), loadComputeTargets()]);
    } catch (error) {
      setStatus("Setup check failed");
      setAuth(String(error));
    }
  }

  function maybeOpenSetupPrompt(nextSetupState: ReturnType<typeof deriveSetupStateFromFallbackModel>, nextFallbackEnabled: boolean) {
    if (!shouldOpenSetupPrompt({
      alreadyPrompted: setupPrompted.current,
      fallbackEnabled: nextFallbackEnabled,
      setupState: nextSetupState,
      skipped: localStorage.getItem(setupSkipKey) === "true",
    })) {
      return;
    }

    setupPrompted.current = true;
    showDetails();
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
      setQueue(unblockFallbackReadyQueue);
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

  async function installFallback(options: { force?: boolean } = {}) {
    if (!isTauri() || fallbackModelBusy) return;

    setFallbackCommandPending(true);
    fallbackEnabledRef.current = true;
    setFallbackEnabled(true);
    setStatus("Installing local fallback");
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

  function skipSetup() {
    localStorage.setItem(setupSkipKey, "true");
    closeDetails();
  }

  async function addPaths(paths: string[]) {
    const firstId = nextRecordingId.current;
    nextRecordingId.current += paths.length;
    const incoming = paths.map((path, index) => ({ id: firstId + index, path }));

    const acceptedCandidates = acceptedRecordingDrops(queueRef.current.map((item) => item.path), incoming);
    const accepted = acceptedCandidates.slice(0, availableQueuedServerSlots(queueRef.current));
    if (paths.length && !acceptedCandidates.length) {
      toast.warning(`Drop ${acceptedFormats} files.`);
      return;
    }
    if (acceptedCandidates.length > accepted.length) {
      toast.warning(
        accepted.length
          ? `Queued ${accepted.length} of ${acceptedCandidates.length} recordings. Connect a server before adding more.`
          : "Server queue is full. Connect a server before adding more recordings.",
      );
    }

    const approved = (
      await Promise.all(
        accepted.map(async (item) => {
          try {
            return { ...item, playbackPath: await allowRecordingPlaybackPath(item.path) };
          } catch {
            return undefined;
          }
        }),
      )
    ).filter((item): item is { id: number; path: string; playbackPath: string } => Boolean(item));

    if (accepted.length && approved.length < accepted.length) {
      toast.warning("Some recordings could not be prepared for playback.");
    }
    if (!approved.length) return;

    const current = queueRef.current;
    const acceptedApprovedIds = new Set(
      acceptedRecordingDrops(current.map((item) => item.path), approved).map((item) => item.id),
    );
    const addable = approved
      .filter((item) => acceptedApprovedIds.has(item.id))
      .slice(0, availableQueuedServerSlots(current));
    if (!addable.length) return;

    const newItems = createQueuedServerRecordingJobs(addable, batchServerQueuedMessage);
    setQueue((current) => [...current, ...newItems]);
    openWorkspace("transcribe");
    selectQueueItem(newItems[newItems.length - 1].id);
  }

  function goToTranscribe() {
    openWorkspace("transcribe");
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
      if (Array.isArray(selected)) await addPaths(selected);
      else if (selected) await addPaths([selected]);
    } catch (error) {
      toast.error(`Picker failed: ${String(error)}`);
    }
  }

  async function transcribeItems(pending: RecordingJobView[]) {
    if (!pending.length || running || !isTauri()) return;
    if (pending.some((item) => item.intent === "recording")) {
      toast.error(batchServerQueuedMessage);
      return;
    }

    setRunning(true);
    setRunningSince(Date.now());
    setStatus(`Transcribing 0/${pending.length}`);

    for (const [index, item] of pending.entries()) {
      if (index === 0) selectQueueItemOnly(item.id);
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

    selectQueueItemOnly(id);
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

  function clearQueue() {
    if (!running) {
      setQueue([]);
      clearTranscriptText();
    }
  }

  const loadHistoryPreviewText = useCallback(
    (entry: TranscriptHistoryEntry) => loadTranscriptPreviewText(entry.outputPath),
    [loadTranscriptPreviewText],
  );
  const {
    closeTranscriptPreview,
    previewEntry,
    previewHistoryEntry,
    previewText,
  } = useTranscriptPreview(loadHistoryPreviewText);

  function hideHistoryEntry(outputPath: string) {
    if (!rememberHiddenHistoryEntry(outputPath)) return;
    if (!forgetHistoryEntry(outputPath)) return;
    clearHistorySelectionIf(outputPath);
    toast.success("Hidden from history");
  }

  async function deleteHistoryEntry(entry: TranscriptHistoryEntry) {
    try {
      await invoke("delete_history_entry_files", {
        outputPath: entry.outputPath,
      });
      if (!rememberHiddenHistoryEntry(entry.outputPath)) return;
      if (!forgetHistoryEntry(entry.outputPath)) return;
      clearHistorySelectionIf(entry.outputPath);
      forgetTranscriptText(entry.outputPath);
      toast.success("Deleted from device");
    } catch (error) {
      toast.error(String(error || "Delete failed"));
    }
  }

  async function recoverHistoryEntry(entry: TranscriptHistoryEntry) {
    const sessionId = entry.name.replace(/^live-/, "");
    try {
      const saved = await recoverLiveSession(sessionId);
      const recovered = savedSessionToTranscriptHistoryEntry(saved);
      if (!recordVisibleHistoryEntries([recovered], "Transcript history could not be saved.")) return;
      forgetHistoryEntry(entry.outputPath);
      clearHistorySelectionIf(entry.outputPath);
      selectHistoryEntry(recovered);
      toast.success("Partial recording recovered");
    } catch (error) {
      toast.error(String(error || "Recovery failed"));
    }
  }

  async function deleteRecoverableHistoryEntry(entry: TranscriptHistoryEntry) {
    const sessionId = entry.name.replace(/^live-/, "");
    try {
      await deleteRecoverableLiveSession(sessionId);
      if (!forgetHistoryEntry(entry.outputPath)) return;
      clearHistorySelectionIf(entry.outputPath);
      forgetTranscriptText(entry.outputPath);
      toast.success("Partial recording deleted");
    } catch (error) {
      toast.error(String(error || "Delete failed"));
    }
  }

  function openHistoryEntry(entry: TranscriptHistoryEntry, origin?: DOMRect) {
    selectHistoryEntry(entry, origin);
    openWorkspace("home");
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
          onCopy={(entry) => void copyTranscript(historyJob(entry))}
          onDelete={(entry) => void deleteHistoryEntry(entry)}
          onDeleteRecoverable={(entry) => void deleteRecoverableHistoryEntry(entry)}
          onHide={hideHistoryEntry}
          onLoadPreviewText={loadHistoryPreviewText}
          onOpen={(entry) => void openAppPath(entry.outputPath)}
          onOpenHelp={() => openWorkspace("help")}
          onPreview={(entry) => void previewHistoryEntry(entry)}
          onReveal={(entry) => void revealPath(entry.outputPath)}
          onRecover={(entry) => void recoverHistoryEntry(entry)}
          onSelect={openHistoryEntry}
          selectedOutputPath={selectedHistoryOutput}
        />
      ) : null}

      {showPolish ? (
        <PolishPanel
          item={selectedItem}
          onLoadText={loadTranscriptText}
          onOpenHelp={() => openWorkspace("help")}
          onPolished={(outputPath, text) => {
            rememberPolishedText(outputPath, text);
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
        onOpenHelp={() => openWorkspace("help")}
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
        onOpenDetails={() => openWorkspace("details")}
        onOpenHelp={() => openWorkspace("help")}
        status={status}
        title={workspace.title}
      />

      {workspaceView === "transcribe" ? (
        <DropHero
          dragging={recordingDrop.dragging}
          onDragLeave={recordingDrop.onDragLeave}
          onDragOver={recordingDrop.onDragOver}
          onDrop={recordingDrop.onDrop}
          onOpenHelp={() => openWorkspace("help")}
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
      <AppSidebar active={activeRail} onAction={openWorkspace} />
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
        onClearLivePasteHotkey={clearLivePasteShortcut}
        onInstallFallback={(options) => void installFallback(options)}
        onOpenFallbackFolder={() => void openFallbackFolder()}
        onPreflightLiveInput={preflightLiveInput}
        onResetLiveHotkey={resetLiveHotkey}
        onOpenChange={onDetailsOpenChange}
        onRemoveFallback={() => void removeFallback()}
        onSetInputDevice={updateInputDevice}
        onSetFallbackEnabled={(enabled) => void setFallbackEnabledSetting(enabled)}
        onVerifyFallback={() => void verifyFallback()}
        onSetLiveCaptureMode={updateLiveCaptureMode}
        onSetLiveHotkey={updateLiveHotkey}
        onSetLiveOverlayEnabled={updateLiveOverlay}
        onSetLivePasteHotkey={updateLivePasteHotkey}
        onSetLocalComputeTarget={(targetId) => void updateLocalComputeTarget(targetId)}
        onSkipSetup={skipSetup}
        onStartLive={startLive}
        onStopLive={stopLive}
        open={detailsOpen}
        serverLabel={serverLabel}
        status={status}
      />
      <HelpSheet
        onOpenChange={onHelpOpenChange}
        open={helpOpen}
      />
      <TranscriptReviewDialog
        elapsedSeconds={elapsedSeconds}
        item={selectedHistoryItem}
        morphOrigin={reviewMorphOrigin}
        onCopy={copyTranscript}
        onOpen={(path) => void openAppPath(path)}
        onOpenChange={(open) => {
          if (!open) closeHistoryReview();
        }}
        onOpenHelp={() => openWorkspace("help")}
        onRetry={(id) => void retryItem(id)}
        onReveal={(path) => void revealPath(path)}
        open={workspaceView === "home" && Boolean(selectedHistoryItem)}
        running={running}
        text={selectedHistoryItem?.output ? transcriptText[selectedHistoryItem.output] : undefined}
      />
      <TranscriptPreviewDialog
        entry={previewEntry}
        onCopy={(entry) => void copyTranscript(historyJob(entry))}
        onOpen={(entry) => void openAppPath(entry.outputPath)}
        onOpenChange={(open) => {
          if (!open) closeTranscriptPreview();
        }}
        onReveal={(entry) => void revealPath(entry.outputPath)}
        text={previewText}
      />
    </SidebarProvider>
  );
}
