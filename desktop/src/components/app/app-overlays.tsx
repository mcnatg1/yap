import type { ComponentProps } from "react";

import { HelpSheet } from "@/components/panels/help-sheet";
import { SettingsSheet } from "@/components/panels/settings-sheet";
import { TranscriptPreviewDialog } from "@/components/transcript-preview-dialog";
import { TranscriptReviewDialog } from "@/components/transcript-review-dialog";
import type { useSettingsControl } from "@/hooks/use-settings-control";
import type { TranscriptHistoryEntry } from "@/history-model";
import type { RecordingJobView } from "@/lib/recording-job";
import type { RailAction } from "@/lib/workspace";

const unavailableHistoryRetry = () => undefined;

type AppOverlaysProps = {
  closeDetails: () => void;
  closeHistoryReview: () => void;
  closeTranscriptPreview: () => void;
  copyTranscript: (item: RecordingJobView) => unknown;
  detailsOpen: boolean;
  helpOpen: boolean;
  historyJob: (entry: TranscriptHistoryEntry) => RecordingJobView;
  historyReviewOpen: boolean;
  onDetailsOpenChange: (open: boolean) => void;
  onHelpOpenChange: (open: boolean) => void;
  openAppPath: (path: string) => unknown;
  openWorkspace: (action: RailAction) => void;
  previewEntry?: TranscriptHistoryEntry;
  previewText?: string;
  revealPath: (path: string) => unknown;
  reviewMorphOrigin?: ComponentProps<typeof TranscriptReviewDialog>["morphOrigin"];
  selectedHistoryItem?: RecordingJobView;
  settings: ReturnType<typeof useSettingsControl>;
  status: string;
  transcriptText: Record<string, string>;
};

export function AppOverlays({
  closeDetails,
  closeHistoryReview,
  closeTranscriptPreview,
  copyTranscript,
  detailsOpen,
  helpOpen,
  historyJob,
  historyReviewOpen,
  onDetailsOpenChange,
  onHelpOpenChange,
  openAppPath,
  openWorkspace,
  previewEntry,
  previewText,
  revealPath,
  reviewMorphOrigin,
  selectedHistoryItem,
  settings,
  status,
  transcriptText,
}: AppOverlaysProps) {
  return (
    <>
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
        onInstallFallback={(options) => void settings.fallback.install(options)}
        onOpenChange={onDetailsOpenChange}
        onOpenFallbackFolder={() => void settings.fallback.openFolder()}
        onPreflightLiveInput={settings.live.preflightInput}
        onRemoveFallback={() => void settings.fallback.remove()}
        onResetLiveHotkey={settings.live.resetHotkey}
        onResetLivePasteHotkey={settings.live.resetPasteHotkey}
        onSetFallbackEnabled={(enabled) => void settings.fallback.setEnabled(enabled)}
        onSetInputDevice={settings.live.updateInputDevice}
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
        onVerifyFallback={() => void settings.fallback.verify()}
        open={detailsOpen}
        serverLabel={settings.serverLabel}
        status={status}
      />
      <HelpSheet onOpenChange={onHelpOpenChange} open={helpOpen} />
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
        onRetry={unavailableHistoryRetry}
        onReveal={(path) => void revealPath(path)}
        open={historyReviewOpen}
        running={false}
        text={selectedHistoryItem?.outputPath ? transcriptText[selectedHistoryItem.outputPath] : undefined}
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
    </>
  );
}
