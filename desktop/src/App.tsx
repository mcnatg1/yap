import { isTauri } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { AppChrome } from "@/components/app/app-chrome";
import { AppSidebar } from "@/components/app/app-sidebar";
import { HelpSheet, projectAppModalState, SettingsSheet } from "@/components/panels/app-sheets";
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
import { useHistoryActions } from "@/hooks/use-history-actions";
import { useLiveHistorySync } from "@/hooks/use-live-history-sync";
import { useRecordingSelection } from "@/hooks/use-recording-selection";
import { useRegisteredPlayback } from "@/hooks/use-registered-playback";
import { useRecordingDrop } from "@/hooks/use-recording-drop";
import { useSettingsControl } from "@/hooks/use-settings-control";
import { useTranscriptFileActions } from "@/hooks/use-transcript-file-actions";
import { useTranscriptPreview } from "@/hooks/use-transcript-preview";
import { useTranscriptText } from "@/hooks/use-transcript-text";
import { useTranscriptHistory } from "@/hooks/use-transcript-history";
import { useWorkspaceNavigation } from "@/hooks/use-workspace-navigation";
import { type TranscriptHistoryEntry } from "@/history";
import {
  acceptedFormats,
  acceptedRecordingDrops,
  audioExtensions,
  isRecordingActive,
  isRecordingFinished,
  isRecordingRetryable,
  isRecordingRunnable,
  queuedServerMessage,
  recordingStatusForStartFailure,
  type RailAction,
  type RecordingJobView,
  workspaceCopy,
} from "@/lib/app-types";
import {
  allowRecordingPlaybackPath,
} from "@/lib/playback-registry";
import { unblockFallbackReadyQueue } from "@/lib/setup-model-state";
import { cn } from "@/lib/utils";
import {
  availableQueuedServerSlots,
  createQueuedServerRecordingJobs,
  nextRecordingQueueId,
  readRecordingQueue,
  writeRecordingQueue,
} from "@/recording-queue";
import { SttInvokeError, startTranscribe } from "@/stt";

export default function App() {
  const initialQueue = useMemo(() => readRecordingQueue(), []);
  const [queue, setQueue] = useState<RecordingJobView[]>(initialQueue);
  const nextRecordingId = useRef(nextRecordingQueueId(initialQueue));
  const [running, setRunning] = useState(false);
  const [runningSince, setRunningSince] = useState<number>();
  const [status, setStatus] = useState("Starting");
  const unblockFallbackReady = useCallback(() => {
    setQueue(unblockFallbackReadyQueue);
  }, []);
  const settingsRefreshRef = useRef<() => Promise<void>>(async () => undefined);
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
    captureNativeHistoryReconciliation,
    forgetHistoryEntry,
    history,
    reconcileHiddenHistory,
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
    onDetailsOpenChange: setDetailsOpen,
    onHelpOpenChange: setHelpOpen,
    openWorkspace: navigateWorkspace,
    railCollapsed,
    setRailCollapsed,
    showDetails: showDetailsNavigation,
    workspaceView,
  } = useWorkspaceNavigation({
    onOpenDetails: () => void settingsRefreshRef.current(),
    onOpenPolish: () => {
      setStatus(isRecordingFinished(selectedItem?.status) ? "Transcript ready" : "Transcribe a file first");
    },
  });
  const setAppModal = useCallback((modal: "settings" | "help" | null) => {
    const next = projectAppModalState(modal);
    setDetailsOpen(next.detailsOpen);
    setHelpOpen(next.helpOpen);
  }, [setDetailsOpen, setHelpOpen]);
  const openWorkspace = useCallback((action: RailAction) => {
    if (action === "details") setAppModal("settings");
    if (action === "help") setAppModal("help");
    navigateWorkspace(action);
  }, [navigateWorkspace, setAppModal]);
  const showDetails = useCallback(() => {
    setAppModal("settings");
    showDetailsNavigation();
  }, [setAppModal, showDetailsNavigation]);
  const onDetailsOpenChange = useCallback((open: boolean) => {
    if (open) setAppModal("settings");
    else setDetailsOpen(false);
  }, [setAppModal, setDetailsOpen]);
  const onHelpOpenChange = useCallback((open: boolean) => {
    if (open) setAppModal("help");
    else setHelpOpen(false);
  }, [setAppModal, setHelpOpen]);
  const onLiveSessionSaved = useCallback((entry: TranscriptHistoryEntry) => {
    selectHistoryEntry(entry);
    openWorkspace("home");
    setStatus("Ready");
    void loadTranscriptText(entry.outputPath).catch(() => undefined);
    if (entry.warning) {
      toast.warning(entry.warning);
    } else {
      toast.success("Live transcript saved");
    }
  }, [loadTranscriptText, openWorkspace, selectHistoryEntry]);
  useLiveHistorySync({
    captureNativeHistoryReconciliation,
    onSaved: onLiveSessionSaved,
    reconcileHiddenHistory,
    recordVisibleHistoryEntries,
  });
  const historyActions = useHistoryActions({
    clearHistorySelectionIf,
    forgetHistoryEntry,
    forgetTranscriptText,
    recordVisibleHistoryEntries,
    rememberHiddenHistoryEntry,
    selectHistoryEntry,
  });
  const settings = useSettingsControl({
    onFallbackReady: unblockFallbackReady,
    onStatusChange: setStatus,
  });
  settingsRefreshRef.current = settings.refresh;
  const workspace = workspaceCopy[workspaceView];
  const showQueue = workspaceView === "transcribe";
  const showHistory = workspaceView === "home";
  const showTranscript = workspaceView === "transcribe" || workspaceView === "polish";
  const showPolish = workspaceView === "polish";

  useEffect(() => {
    if (settings.setupPromptRequest) showDetails();
  }, [settings.setupPromptRequest, showDetails]);

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
    if (selectedItem?.output && !Object.prototype.hasOwnProperty.call(transcriptText, selectedItem.output)) {
      void loadTranscriptText(selectedItem.output).catch(() => toast.error("Preview unavailable"));
    }
  }, [selectedItem?.output, transcriptText]);

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
          ? `Queued ${accepted.length} of ${acceptedCandidates.length} recordings. Connect to your organization's transcription server before adding more.`
          : "The organization server queue is full. Connect before adding more recordings.",
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

    const newItems = createQueuedServerRecordingJobs(addable);
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
      toast.error(queuedServerMessage);
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
          onDelete={(entry) => void historyActions.deleteHistoryEntry(entry)}
          onDeleteRecoverable={(entry) => void historyActions.deleteRecoverableHistoryEntry(entry)}
          onHide={historyActions.hideHistoryEntry}
          onLoadPreviewText={loadHistoryPreviewText}
          onOpen={(entry) => void openAppPath(entry.outputPath)}
          onOpenHelp={() => openWorkspace("help")}
          onPreview={(entry) => void previewHistoryEntry(entry)}
          onReveal={(entry) => void revealPath(entry.outputPath)}
          onRecover={(entry) => void historyActions.recoverHistoryEntry(entry)}
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
        auth={settings.auth}
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
        auth={settings.auth}
        busy={settings.busy}
        fallbackActionPending={settings.fallback.actionPending}
        fallbackModel={settings.fallback.model}
        liveBusy={settings.live.busy}
        liveInputDevices={settings.live.inputDevices}
        liveSettingsError={settings.live.settingsError}
        liveView={settings.live.view}
        localComputeTargets={settings.compute.targets}
        onCancelFallbackInstall={() => void settings.fallback.cancelInstall()}
        onClearLiveHotkey={settings.live.clearShortcut}
        onClearLivePasteHotkey={settings.live.clearPasteShortcut}
        onInstallFallback={(options) => void settings.fallback.install(options)}
        onOpenFallbackFolder={() => void settings.fallback.openFolder()}
        onPreflightLiveInput={settings.live.preflightInput}
        onResetLiveHotkey={settings.live.resetHotkey}
        onOpenChange={onDetailsOpenChange}
        onRemoveFallback={() => void settings.fallback.remove()}
        onSetInputDevice={settings.live.updateInputDevice}
        onSetFallbackEnabled={(enabled) => void settings.fallback.setEnabled(enabled)}
        onVerifyFallback={() => void settings.fallback.verify()}
        onSetLiveCaptureMode={settings.live.updateCaptureMode}
        onSetLiveHotkey={settings.live.updateHotkey}
        onSetLiveOverlayEnabled={settings.live.updateOverlay}
        onSetLivePasteHotkey={settings.live.updatePasteHotkey}
        onSetLocalComputeTarget={(targetId) => void settings.compute.updateTarget(targetId)}
        onSkipSetup={() => {
          settings.skipSetup();
          closeDetails();
        }}
        onStartLive={settings.live.start}
        onStopLive={settings.live.stop}
        open={detailsOpen}
        serverLabel={settings.serverLabel}
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
