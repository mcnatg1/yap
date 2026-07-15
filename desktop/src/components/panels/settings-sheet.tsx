import { useEffect, useState } from "react";

import { AboutSettingsSection } from "@/components/settings/about-settings-section";
import { GeneralSettingsSection } from "@/components/settings/general-settings-section";
import {
  SettingsNavigation,
  settingsSectionTitle,
  type SettingsSection,
} from "@/components/settings/settings-navigation";
import {
  liveSettingsLocked,
  projectFallbackLifecycle,
  projectLiveOverlayAction,
  type FallbackLifecycleActionId,
} from "@/components/settings/settings-lifecycle";
import { SystemSettingsSection } from "@/components/settings/system-settings-section";
import { useServerSettingsDraft } from "@/components/settings/use-server-settings-draft";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import type { LiveCaptureMode, LiveInputDeviceView, LiveSessionView } from "@/lib/live-session";
import type { FallbackModelView, LocalComputeTargetView } from "@/lib/setup-model";

export type SettingsSheetProps = {
  auth: string;
  busy: boolean;
  fallbackActionPending: boolean;
  fallbackModel: FallbackModelView | null;
  liveBusy: boolean;
  liveInputDevices: LiveInputDeviceView[];
  liveSettingsError: string;
  liveView: LiveSessionView;
  localComputeTargets: LocalComputeTargetView[];
  onCancelFallbackInstall: () => void;
  onInstallFallback: (options?: { force?: boolean }) => void;
  onOpenChange: (open: boolean) => void;
  onOpenFallbackFolder: () => void;
  onPreflightLiveInput: () => void;
  onRemoveFallback: () => void;
  onResetLiveHotkey: () => void;
  onResetLivePasteHotkey: () => void;
  onSetFallbackEnabled: (enabled: boolean) => void;
  onSetInputDevice: (deviceId?: string) => void;
  onSetLiveCaptureMode: (captureMode: LiveCaptureMode) => void;
  onSetLiveHotkey: () => void;
  onSetLiveOverlayEnabled: (enabled: boolean) => void;
  onSetLivePasteHotkey: () => void;
  onSetLocalComputeTarget: (targetId: string) => void;
  onSkipSetup: () => void;
  onStartLive: () => void;
  onStopLive: () => void;
  onVerifyFallback: () => void;
  open: boolean;
  serverLabel: string;
  status: string;
};

export function SettingsSheet({
  auth,
  busy,
  fallbackActionPending,
  fallbackModel,
  liveBusy,
  liveInputDevices,
  liveSettingsError,
  liveView,
  localComputeTargets,
  onCancelFallbackInstall,
  onInstallFallback,
  onOpenChange,
  onOpenFallbackFolder,
  onPreflightLiveInput,
  onRemoveFallback,
  onResetLiveHotkey,
  onResetLivePasteHotkey,
  onSetFallbackEnabled,
  onSetInputDevice,
  onSetLiveCaptureMode,
  onSetLiveHotkey,
  onSetLiveOverlayEnabled,
  onSetLivePasteHotkey,
  onSetLocalComputeTarget,
  onSkipSetup,
  onStartLive,
  onStopLive,
  onVerifyFallback,
  open,
  serverLabel,
  status,
}: SettingsSheetProps) {
  const fallbackStatus = fallbackModel?.status;
  const [section, setSection] = useState<SettingsSection>(
    fallbackStatus && fallbackStatus !== "ready" ? "system" : "general",
  );
  const [confirmRemoveOpen, setConfirmRemoveOpen] = useState(false);
  const server = useServerSettingsDraft(open);
  const liveActive = liveSettingsLocked(liveView.status);
  const liveOverlayAction = projectLiveOverlayAction(liveView.status, liveBusy);
  const fallbackLifecycle = projectFallbackLifecycle(fallbackModel, {
    commandPending: fallbackActionPending,
    liveStatus: liveView.status,
  });

  useEffect(() => {
    if (open && fallbackStatus && fallbackStatus !== "ready") {
      setSection("system");
    }
  }, [fallbackStatus, open]);

  function runFallbackAction(actionId: FallbackLifecycleActionId) {
    switch (actionId) {
      case "install":
      case "repair":
      case "retry":
        onInstallFallback();
        return;
      case "reinstall":
        onInstallFallback({ force: true });
        return;
      case "cancel":
        onCancelFallbackInstall();
        return;
      case "open-folder":
        onOpenFallbackFolder();
        return;
      case "verify":
        onVerifyFallback();
        return;
      case "disable":
        onSetFallbackEnabled(false);
        return;
      case "enable":
        onSetFallbackEnabled(true);
        return;
      case "remove":
        setConfirmRemoveOpen(true);
        return;
    }
  }

  return (
    <AlertDialog onOpenChange={setConfirmRemoveOpen} open={confirmRemoveOpen}>
      <Dialog onOpenChange={onOpenChange} open={open}>
        <DialogContent
          className="grid h-[min(720px,calc(100vh-40px))] w-[1120px] gap-0 overflow-hidden rounded-2xl border-0 bg-background p-0 shadow-[0_24px_80px_rgba(0,0,0,0.28)] !max-w-[calc(100vw-40px)]"
          showCloseButton
        >
          <DialogTitle className="sr-only">Settings</DialogTitle>
          <DialogDescription className="sr-only">Yap settings.</DialogDescription>
          <div className="grid min-h-0 grid-cols-[260px_minmax(0,1fr)]">
            <SettingsNavigation onSelect={setSection} section={section} />
            <div className="min-h-0 overflow-y-auto p-10">
              <div className="mx-auto grid max-w-[820px] gap-8">
                <header>
                  <h2 className="text-balance text-3xl font-medium tracking-normal">
                    {settingsSectionTitle(section)}
                  </h2>
                </header>

                {section === "general" ? (
                  <GeneralSettingsSection
                    liveActive={liveActive}
                    liveBusy={liveBusy}
                    liveInputDevices={liveInputDevices}
                    liveOverlayAction={liveOverlayAction}
                    liveSettingsError={liveSettingsError}
                    liveView={liveView}
                    onPreflightLiveInput={onPreflightLiveInput}
                    onResetLiveHotkey={onResetLiveHotkey}
                    onResetLivePasteHotkey={onResetLivePasteHotkey}
                    onSetInputDevice={onSetInputDevice}
                    onSetLiveCaptureMode={onSetLiveCaptureMode}
                    onSetLiveHotkey={onSetLiveHotkey}
                    onSetLiveOverlayEnabled={onSetLiveOverlayEnabled}
                    onSetLivePasteHotkey={onSetLivePasteHotkey}
                    onStartLive={onStartLive}
                    onStopLive={onStopLive}
                  />
                ) : null}

                {section === "system" ? (
                  <SystemSettingsSection
                    busy={busy}
                    fallbackLifecycle={fallbackLifecycle}
                    fallbackLocked={liveActive}
                    liveActive={liveActive}
                    localComputeTargets={localComputeTargets}
                    onFallbackAction={runFallbackAction}
                    onSetLocalComputeTarget={onSetLocalComputeTarget}
                    server={server}
                  />
                ) : null}

                {section === "about" ? (
                  <AboutSettingsSection
                    auth={auth}
                    canSkipSetup={Boolean(fallbackStatus && fallbackStatus !== "ready")}
                    onSkipSetup={onSkipSetup}
                    serverLabel={serverLabel}
                    skipSetupDisabled={busy || fallbackActionPending}
                    status={status}
                  />
                ) : null}
              </div>
            </div>
          </div>
        </DialogContent>
      </Dialog>
      <AlertDialogContent onClick={(event) => event.stopPropagation()}>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove local fallback?</AlertDialogTitle>
          <AlertDialogDescription>
            Local live fallback will be unavailable until reinstalled.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            disabled={fallbackActionPending || liveActive}
            onClick={onRemoveFallback}
          >
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
