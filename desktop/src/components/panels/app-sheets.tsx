import { SealCheck as BadgeCheck } from "@phosphor-icons/react/SealCheck";
import { Copy } from "@phosphor-icons/react/Copy";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { FolderSimple as FolderOutput } from "@phosphor-icons/react/FolderSimple";
import { LockKey as LockKeyhole } from "@phosphor-icons/react/LockKey";
import { Microphone as Mic } from "@phosphor-icons/react/Microphone";
import { HardDrives as Server } from "@phosphor-icons/react/HardDrives";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import { CloudArrowUp as UploadCloud } from "@phosphor-icons/react/CloudArrowUp";
import type { ComponentType, ReactNode } from "react";
import { useEffect, useId, useState } from "react";

import { StatusRow } from "@/components/app/status-row";
import { Button } from "@/components/ui/button";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import {
  type FallbackModelView,
  type FallbackModelStatus,
  liveStatusLabel,
  type LocalComputeTargetView,
  type LiveCaptureMode,
  type LiveInputDeviceView,
  type LiveSessionStatus,
  type LiveSessionView,
} from "@/lib/app-types";
import { cn } from "@/lib/utils";

type SettingsSection = "general" | "system" | "about";

const settingsSections: { id: SettingsSection; icon: ComponentType<{ className?: string }>; label: string }[] = [
  { id: "general", icon: Mic, label: "General" },
  { id: "system", icon: Server, label: "System" },
  { id: "about", icon: BadgeCheck, label: "About" },
];

export type FallbackLifecycleActionId =
  | "install"
  | "cancel"
  | "open-folder"
  | "reinstall"
  | "verify"
  | "disable"
  | "remove"
  | "repair"
  | "enable"
  | "retry";

type FallbackLifecycleAction = {
  disabled: boolean;
  id: FallbackLifecycleActionId;
  label: string;
};

export type FallbackLifecycleProjection = {
  detail?: string;
  primaryAction?: FallbackLifecycleAction;
  secondaryActions: FallbackLifecycleAction[];
  value: string;
};

const fallbackCommandLabels: Record<FallbackLifecycleActionId, string> = {
  install: "Install",
  cancel: "Cancel",
  "open-folder": "Open folder",
  reinstall: "Reinstall",
  verify: "Verify",
  disable: "Disable",
  remove: "Remove",
  repair: "Repair",
  enable: "Enable",
  retry: "Retry",
};

function liveLocksFallbackActions(status: LiveSessionStatus) {
  return ["armed", "listening", "speaking", "settling", "saving"].includes(status);
}

function fallbackDownloadLabel(view: FallbackModelView) {
  const percent = typeof view.progressPercent === "number" ? Math.max(0, Math.min(100, Math.round(view.progressPercent))) : null;
  return percent === null ? "Downloading" : `Downloading ${percent}%`;
}

function fallbackDownloadDetail(view: FallbackModelView) {
  if (typeof view.speedMbps === "number" && Number.isFinite(view.speedMbps) && view.speedMbps > 0) {
    return `${view.speedMbps.toFixed(view.speedMbps >= 10 ? 0 : 1)} Mbps`;
  }
  return view.message ?? undefined;
}

function createAction(
  id: FallbackLifecycleActionId,
  disabled: boolean,
) {
  return {
    disabled,
    id,
    label: fallbackCommandLabels[id],
  } satisfies FallbackLifecycleAction;
}

export function projectFallbackLifecycle(
  fallbackModel: FallbackModelView | null,
  options: {
    commandPending: boolean;
    liveStatus: LiveSessionStatus;
  },
): FallbackLifecycleProjection {
  if (!fallbackModel) {
    return {
      secondaryActions: [],
      value: "Checking",
    };
  }

  const liveActive = liveLocksFallbackActions(options.liveStatus);
  const toggleDisabled = options.commandPending || liveActive;
  const removeDisabled = options.commandPending || liveActive;
  const verifyDisabled = options.commandPending || liveActive;
  const installDisabled = options.commandPending || liveActive;
  const openFolder = createAction("open-folder", false);

  const secondaryActions = (status: FallbackModelStatus) => {
    switch (status) {
      case "missing":
      case "downloading":
      case "verifying":
        return [openFolder];
      case "ready":
        return [
          createAction("verify", verifyDisabled),
          createAction("disable", toggleDisabled),
          createAction("remove", removeDisabled),
          openFolder,
        ];
      case "corrupted":
        return [createAction("remove", removeDisabled), openFolder];
      case "disabled":
        return [createAction("remove", removeDisabled), openFolder];
      case "error":
        return [createAction("remove", removeDisabled), openFolder];
    }
  };

  switch (fallbackModel.status) {
    case "missing":
      return {
        detail: fallbackModel.label,
        primaryAction: createAction("install", installDisabled),
        secondaryActions: secondaryActions("missing"),
        value: "Not installed",
      };
    case "downloading":
      return {
        detail: fallbackDownloadDetail(fallbackModel),
        primaryAction: createAction("cancel", options.commandPending || fallbackModel.status !== "downloading"),
        secondaryActions: secondaryActions("downloading"),
        value: fallbackDownloadLabel(fallbackModel),
      };
    case "verifying":
      return {
        detail: fallbackModel.message ?? fallbackModel.label,
        secondaryActions: secondaryActions("verifying"),
        value: "Verifying files",
      };
    case "ready":
      return {
        detail: fallbackModel.label,
        primaryAction: createAction("reinstall", installDisabled),
        secondaryActions: secondaryActions("ready"),
        value: "Ready",
      };
    case "corrupted":
      return {
        detail: fallbackModel.message ?? fallbackModel.label,
        primaryAction: createAction("repair", installDisabled),
        secondaryActions: secondaryActions("corrupted"),
        value: "Files failed verification.",
      };
    case "disabled":
      return {
        detail: fallbackModel.label,
        primaryAction: createAction("enable", toggleDisabled),
        secondaryActions: secondaryActions("disabled"),
        value: "Disabled",
      };
    case "error":
      return {
        detail: fallbackModel.message ?? fallbackModel.label,
        primaryAction: createAction("retry", installDisabled),
        secondaryActions: secondaryActions("error"),
        value: "Needs attention",
      };
  }
}

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
  onClearLiveHotkey,
  onOpenChange,
  onInstallFallback,
  onOpenFallbackFolder,
  onPreflightLiveInput,
  onResetLiveHotkey,
  onRemoveFallback,
  onSetInputDevice,
  onSetFallbackEnabled,
  onVerifyFallback,
  onSetLiveCaptureMode,
  onSetLiveHotkey,
  onSetLiveOverlayEnabled,
  onSetLocalComputeTarget,
  onSkipSetup,
  onStartLive,
  onStopLive,
  open,
  serverLabel,
  status,
}: {
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
  onClearLiveHotkey: () => void;
  onOpenChange: (open: boolean) => void;
  onInstallFallback: () => void;
  onOpenFallbackFolder: () => void;
  onPreflightLiveInput: () => void;
  onResetLiveHotkey: () => void;
  onRemoveFallback: () => void;
  onSetInputDevice: (deviceId?: string) => void;
  onSetFallbackEnabled: (enabled: boolean) => void;
  onVerifyFallback: () => void;
  onSetLiveCaptureMode: (captureMode: LiveCaptureMode) => void;
  onSetLiveHotkey: (hotkey: string) => void;
  onSetLiveOverlayEnabled: (enabled: boolean) => void;
  onSetLocalComputeTarget: (targetId: string) => void;
  onSkipSetup: () => void;
  onStartLive: () => void;
  onStopLive: () => void;
  open: boolean;
  serverLabel: string;
  status: string;
}) {
  const liveActive = ["armed", "listening", "speaking", "settling"].includes(liveView.status);
  const fallbackLocked = liveLocksFallbackActions(liveView.status);
  const micLabelId = useId();
  const computeLabelId = useId();
  const modeLabelId = useId();
  const [section, setSection] = useState<SettingsSection>(fallbackModel && fallbackModel.status !== "ready" ? "system" : "general");
  const [confirmRemoveOpen, setConfirmRemoveOpen] = useState(false);
  const [hotkeyDraft, setHotkeyDraft] = useState(liveView.hotkey);
  const selectedComputeTarget = localComputeTargets.find((target) => target.selected)?.id ?? "auto";
  const fallbackLifecycle = projectFallbackLifecycle(fallbackModel, {
    commandPending: fallbackActionPending,
    liveStatus: liveView.status,
  });

  useEffect(() => {
    setHotkeyDraft(liveView.hotkey);
  }, [liveView.hotkey]);

  useEffect(() => {
    if (open && fallbackModel && fallbackModel.status !== "ready") {
      setSection("system");
    }
  }, [fallbackModel, open]);

  function runFallbackAction(actionId: FallbackLifecycleActionId) {
    switch (actionId) {
      case "install":
      case "reinstall":
      case "repair":
      case "retry":
        onInstallFallback();
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
            <aside className="flex min-h-0 flex-col border-r bg-muted/45 p-5">
              <div className="mb-4 text-xs font-medium uppercase tracking-[0.14em] text-muted-foreground">
                Settings
              </div>
              <nav className="grid gap-1">
                {settingsSections.map((item) => {
                  const Icon = item.icon;
                  return (
                    <button
                      className={cn(
                        "flex h-11 items-center gap-3 rounded-lg px-3 text-left text-sm font-medium transition-[background-color,color,scale] duration-150 ease-out active:scale-[0.96]",
                        section === item.id ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:bg-background/60 hover:text-foreground",
                      )}
                      key={item.id}
                      onClick={() => setSection(item.id)}
                      type="button"
                    >
                      <Icon className="size-5 shrink-0" />
                      {item.label}
                    </button>
                  );
                })}
              </nav>
              <div className="mt-auto text-xs text-muted-foreground">Yap</div>
            </aside>
            <div className="min-h-0 overflow-y-auto p-10">
              <div className="mx-auto grid max-w-[820px] gap-8">
                <header>
                  <h2 className="text-balance text-3xl font-medium tracking-normal">
                    {section === "general" ? "General" : section === "system" ? "System" : "About"}
                  </h2>
                </header>

                {section === "general" ? (
                  <SettingsGroup>
                    <SettingsRow
                      action={
                        <Button disabled={liveBusy || liveActive || hotkeyDraft === liveView.hotkey} onClick={() => onSetLiveHotkey(hotkeyDraft)} type="button" variant="secondary">
                          Apply
                        </Button>
                      }
                      detail={liveActive ? "Stop live first." : "Used to start dictation."}
                      label="Shortcut"
                      value={liveView.hotkey || "Off"}
                    >
                      <Input
                        className="max-w-[260px]"
                        disabled={liveBusy || liveActive}
                        onKeyDown={(event) => {
                          if (event.key === "Enter") onSetLiveHotkey(hotkeyDraft);
                        }}
                        placeholder="Ctrl+Shift+Space"
                        value={hotkeyDraft}
                        onChange={(event) => setHotkeyDraft(event.currentTarget.value)}
                      />
                    </SettingsRow>
                    <SettingsRow
                      action={
                        <Button disabled={liveBusy || liveActive} onClick={onPreflightLiveInput} type="button" variant="secondary">
                          Check
                        </Button>
                      }
                      detail={liveActive ? "Stop live before changing microphones." : "Auto falls back to the system default."}
                      label="Microphone"
                      value={liveView.inputDeviceLabel || "System default"}
                    >
                      <Select
                        disabled={liveBusy || liveActive}
                        onValueChange={(value) => onSetInputDevice(value === "default" ? undefined : value)}
                        value={liveView.inputDeviceId ?? "default"}
                      >
                        <SelectTrigger aria-labelledby={micLabelId} className="w-full max-w-[340px]">
                          <SelectValue placeholder="System default" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value="default">System default</SelectItem>
                            {liveInputDevices.map((device) => (
                              <SelectItem key={device.id} value={device.id}>
                                {device.label}{device.isDefault ? " (default)" : ""}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </SettingsRow>
                    <SettingsRow
                      detail={liveActive ? "Stop live before changing mode." : "Hold for push-to-talk, toggle for hands-free."}
                      label="Mode"
                      value={liveView.captureMode === "pushToTalk" ? "Hold" : "Toggle"}
                    >
                      <Label className="sr-only" id={modeLabelId}>
                        Mode
                      </Label>
                      <ToggleGroup
                        aria-labelledby={modeLabelId}
                        className="grid max-w-[260px] grid-cols-2"
                        disabled={liveBusy || liveActive}
                        onValueChange={(value) => {
                          if (value) onSetLiveCaptureMode(value as LiveCaptureMode);
                        }}
                        type="single"
                        value={liveView.captureMode}
                      >
                        <ToggleGroupItem value="pushToTalk">Hold</ToggleGroupItem>
                        <ToggleGroupItem value="toggle">Toggle</ToggleGroupItem>
                      </ToggleGroup>
                    </SettingsRow>
                    <SettingsRow
                      action={
                        <Button disabled={liveBusy} onClick={liveActive ? onStopLive : onStartLive} type="button">
                          <Mic data-icon="inline-start" />
                          {liveActive ? "Stop" : "Start"}
                        </Button>
                      }
                      detail={liveView.error || liveSettingsError || "Small overlay stays available for live dictation."}
                      label="Overlay"
                      value={liveStatusLabel(liveView.status)}
                    >
                      <div className="flex flex-wrap gap-2">
                        <Button disabled={liveBusy || liveActive} onClick={() => onSetLiveOverlayEnabled(liveView.visibility === "hidden")} type="button" variant="secondary">
                          {liveView.visibility === "hidden" ? "Show" : "Hide"}
                        </Button>
                        <Button disabled={liveBusy || liveActive} onClick={onResetLiveHotkey} type="button" variant="ghost">
                          Reset shortcut
                        </Button>
                        <Button disabled={liveBusy || liveActive || !liveView.hotkey} onClick={onClearLiveHotkey} type="button" variant="ghost">
                          Clear shortcut
                        </Button>
                      </div>
                    </SettingsRow>
                  </SettingsGroup>
                ) : null}

                {section === "system" ? (
                  <SettingsGroup>
                    <SettingsRow
                      detail={liveActive ? "Stop live before changing compute." : "Local live uses the CPU runtime. Server owns GPU routing."}
                      label="Compute"
                      value={localComputeTargets.find((target) => target.selected)?.label ?? "Auto"}
                    >
                      <Label className="sr-only" id={computeLabelId}>
                        Compute
                      </Label>
                      <Select
                        disabled={busy || fallbackLocked}
                        onValueChange={onSetLocalComputeTarget}
                        value={selectedComputeTarget}
                      >
                        <SelectTrigger aria-labelledby={computeLabelId} className="w-full max-w-[360px]">
                          <SelectValue placeholder="Auto" />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            {localComputeTargets.map((target) => (
                              <SelectItem key={target.id} value={target.id}>
                                {target.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </SettingsRow>
                    <SettingsRow
                      detail={fallbackLifecycle.detail}
                      label="Local fallback"
                      value={fallbackLifecycle.value}
                    >
                      <div className="flex flex-wrap justify-end gap-2">
                        {fallbackLifecycle.primaryAction ? (
                          <Button
                            disabled={fallbackLifecycle.primaryAction.disabled}
                            onClick={() => runFallbackAction(fallbackLifecycle.primaryAction!.id)}
                            type="button"
                          >
                            {fallbackLifecycle.primaryAction.label}
                          </Button>
                        ) : null}
                        {fallbackLifecycle.secondaryActions.map((action) => (
                          <Button
                            disabled={action.disabled}
                            key={action.id}
                            onClick={() => runFallbackAction(action.id)}
                            type="button"
                            variant={action.id === "open-folder" ? "ghost" : "secondary"}
                          >
                            {action.label}
                          </Button>
                        ))}
                      </div>
                    </SettingsRow>
                  </SettingsGroup>
                ) : null}

                {section === "about" ? (
                  <SettingsGroup>
                    <StatusRow icon={BadgeCheck} label="Status" value={status} />
                    <StatusRow icon={Server} label="Server" value={serverLabel} />
                    <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
                    <StatusRow icon={FolderOutput} label="Output" value="Source folder" />
                    {fallbackModel?.status !== "ready" ? (
                      <Button disabled={busy || fallbackActionPending} onClick={onSkipSetup} type="button" variant="secondary">
                        Skip setup prompt
                      </Button>
                    ) : null}
                  </SettingsGroup>
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
            disabled={fallbackActionPending || fallbackLocked}
            onClick={onRemoveFallback}
          >
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function SettingsGroup({ children }: { children: ReactNode }) {
  return <div className="rounded-2xl bg-muted/35 p-6 shadow-[0_0_0_1px_rgba(0,0,0,0.04)]">{children}</div>;
}

function SettingsRow({
  action,
  children,
  detail,
  label,
  value,
}: {
  action?: ReactNode;
  children?: ReactNode;
  detail?: string;
  label: string;
  value: string;
}) {
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_minmax(260px,360px)] gap-4 border-b py-5 first:pt-0 last:border-b-0 last:pb-0">
      <div className="min-w-0 text-pretty">
        <div className="font-medium">{label}</div>
        <div className="mt-1 break-words text-sm text-foreground/80">{value}</div>
        {detail ? <div className="mt-1 break-words text-xs text-muted-foreground">{detail}</div> : null}
      </div>
      <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">
        {children}
        {action}
      </div>
    </div>
  );
}

export function HelpSheet({ onOpenChange, open }: { onOpenChange: (open: boolean) => void; open: boolean }) {
  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent className="w-[min(420px,calc(100vw-24px))] sm:max-w-md" side="right">
        <SheetHeader>
          <SheetTitle>Help</SheetTitle>
          <SheetDescription>Core controls.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-3 px-4">
          <StatusRow icon={UploadCloud} label="Add files" value="Drag files in, or click Drop files here." wrap />
          <StatusRow
            icon={Sparkles}
            label="Transcribe"
            value="Saves beside the source when allowed, otherwise to local Yap transcripts."
            wrap
          />
          <StatusRow icon={Copy} label="Copy" value="Copies transcript text after a file finishes." wrap />
          <StatusRow icon={FolderOpen} label="Reveal" value="Shows the saved transcript in File Explorer." wrap />
        </div>
        <SheetFooter>
          <SheetClose asChild>
            <Button type="button" variant="outline">
              Close
            </Button>
          </SheetClose>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}
