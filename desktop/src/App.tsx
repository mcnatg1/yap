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
import { useHistoryActions } from "@/hooks/use-history-actions";
import { useImportedRecordingQueue } from "@/hooks/use-imported-recording-queue";
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
  audioExtensions,
  isRecordingFinished,
  type RailAction,
  type RecordingJobView,
  workspaceCopy,
} from "@/lib/app-types";
import { cn } from "@/lib/utils";
import {
  projectHistoryPlaybackAdmission,
  type HistoryPlaybackAdmissions,
} from "@/lib/playback-registry";

const ignoreFallbackReady = () => undefined;
const unavailableRecordingRetry = () => undefined;

function withHistoryPlaybackAdmission(
  item: RecordingJobView | undefined,
  entry: TranscriptHistoryEntry | undefined,
  admissions: HistoryPlaybackAdmissions,
) {
  if (!item || !entry || item.output !== entry.outputPath) return item;
  const admission = projectHistoryPlaybackAdmission(entry, admissions);
  if (!admission) return item;
  return {
    ...item,
    playbackByteLength: admission.byteLength,
    playbackPath: admission.playbackPath,
  };
}

export default function App() {
  const [status, setStatus] = useState("Starting");
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
    addPaths: enqueueImportedPaths,
    clearQueue,
    queue,
    removeItem,
    setQueue,
  } = useImportedRecordingQueue(clearTranscriptText);
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
  const {
    clearHistorySelectionIf,
    closeHistoryReview,
    historyJob,
    reviewMorphOrigin,
    selectHistoryEntry,
    selectQueueItem,
    selectedHistoryEntry,
    selectedHistoryItem: selectedHistoryItemWithoutPlaybackMetadata,
    selectedHistoryOutput,
    selectedId,
    selectedItem: selectedItemWithoutPlaybackMetadata,
  } = useRecordingSelection({ history, queue });
  const {
    historyPlaybackAdmissions,
  } = useRegisteredPlayback(queue, setQueue, history, selectedHistoryEntry);
  const selectedHistoryItem = useMemo(
    () => withHistoryPlaybackAdmission(
      selectedHistoryItemWithoutPlaybackMetadata,
      selectedHistoryEntry,
      historyPlaybackAdmissions,
    ),
    [
      historyPlaybackAdmissions,
      selectedHistoryEntry,
      selectedHistoryItemWithoutPlaybackMetadata,
    ],
  );
  const selectedItem = useMemo(
    () => selectedHistoryEntry
      ? withHistoryPlaybackAdmission(
          selectedItemWithoutPlaybackMetadata,
          selectedHistoryEntry,
          historyPlaybackAdmissions,
        )
      : selectedItemWithoutPlaybackMetadata,
    [
      historyPlaybackAdmissions,
      selectedHistoryEntry,
      selectedItemWithoutPlaybackMetadata,
    ],
  );
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
      setStatus(isRecordingFinished(selectedItem?.status) ? "Transcript ready" : "Select a finished transcript first");
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
  const addPaths = useCallback(async (paths: string[]) => {
    const selectedQueueId = await enqueueImportedPaths(paths);
    if (selectedQueueId === undefined) return;
    openWorkspace("transcribe");
    selectQueueItem(selectedQueueId);
  }, [enqueueImportedPaths, openWorkspace, selectQueueItem]);
  const recordingDrop = useRecordingDrop(addPaths);
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
    onFallbackReady: ignoreFallbackReady,
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
    if (selectedItem?.output && !Object.prototype.hasOwnProperty.call(transcriptText, selectedItem.output)) {
      void loadTranscriptText(selectedItem.output).catch(() => toast.error("Preview unavailable"));
    }
  }, [selectedItem?.output, transcriptText]);

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
          onClear={clearQueue}
          onRemove={removeItem}
          onReveal={(path) => void revealPath(path)}
          onSelect={selectQueueItem}
          queue={queue}
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
        elapsedSeconds={0}
        item={selectedItem}
        onCopy={copyTranscript}
        onOpen={(path) => void openAppPath(path)}
        onOpenHelp={() => openWorkspace("help")}
        onRetry={unavailableRecordingRetry}
        onReveal={(path) => void revealPath(path)}
        running={false}
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
        elapsedSeconds={0}
        item={selectedHistoryItem}
        morphOrigin={reviewMorphOrigin}
        onCopy={copyTranscript}
        onOpen={(path) => void openAppPath(path)}
        onOpenChange={(open) => {
          if (!open) closeHistoryReview();
        }}
        onOpenHelp={() => openWorkspace("help")}
        onRetry={unavailableRecordingRetry}
        onReveal={(path) => void revealPath(path)}
        open={workspaceView === "home" && Boolean(selectedHistoryItem)}
        running={false}
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
