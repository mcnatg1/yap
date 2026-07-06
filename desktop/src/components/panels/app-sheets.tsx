import { SealCheck as BadgeCheck } from "@phosphor-icons/react/SealCheck";
import { Copy } from "@phosphor-icons/react/Copy";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { FolderSimple as FolderOutput } from "@phosphor-icons/react/FolderSimple";
import { LockKey as LockKeyhole } from "@phosphor-icons/react/LockKey";
import { Keyboard } from "@phosphor-icons/react/Keyboard";
import { Microphone as Mic } from "@phosphor-icons/react/Microphone";
import { HardDrives as Server } from "@phosphor-icons/react/HardDrives";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import { Trash as Trash2 } from "@phosphor-icons/react/Trash";
import { CloudArrowUp as UploadCloud } from "@phosphor-icons/react/CloudArrowUp";
import { useEffect, useId, useState } from "react";

import { StatusRow } from "@/components/app/status-row";
import { Button } from "@/components/ui/button";
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
  liveRouteLabel,
  liveStatusLabel,
  type LiveCaptureMode,
  type LiveInputDeviceView,
  type LiveSessionView,
} from "@/lib/app-types";

export function SettingsSheet({
  auth,
  busy,
  engineReady,
  engineBinaryStatus,
  fallbackEnabled,
  liveBusy,
  liveInputDevices,
  liveSettingsError,
  liveView,
  model,
  modelInstalled,
  onClearLiveHotkey,
  onOpenChange,
  onInstallFallback,
  onPreflightLiveInput,
  onResetLiveHotkey,
  onRemoveFallback,
  onSetInputDevice,
  onSetFallbackEnabled,
  onSetLiveCaptureMode,
  onSetLiveHotkey,
  onSetLiveOverlayEnabled,
  onSkipSetup,
  onStartLive,
  onStopLive,
  open,
  serverLabel,
  setupLabel,
  setupRoot,
  status,
}: {
  auth: string;
  busy: boolean;
  engineReady: boolean;
  engineBinaryStatus: string;
  fallbackEnabled: boolean;
  liveBusy: boolean;
  liveInputDevices: LiveInputDeviceView[];
  liveSettingsError: string;
  liveView: LiveSessionView;
  model: string;
  modelInstalled: boolean;
  onClearLiveHotkey: () => void;
  onOpenChange: (open: boolean) => void;
  onInstallFallback: () => void;
  onPreflightLiveInput: () => void;
  onResetLiveHotkey: () => void;
  onRemoveFallback: () => void;
  onSetInputDevice: (deviceId?: string) => void;
  onSetFallbackEnabled: (enabled: boolean) => void;
  onSetLiveCaptureMode: (captureMode: LiveCaptureMode) => void;
  onSetLiveHotkey: (hotkey: string) => void;
  onSetLiveOverlayEnabled: (enabled: boolean) => void;
  onSkipSetup: () => void;
  onStartLive: () => void;
  onStopLive: () => void;
  open: boolean;
  serverLabel: string;
  setupLabel: string;
  setupRoot: string;
  status: string;
}) {
  const canRemove = modelInstalled || engineBinaryStatus === "Installed";
  const liveActive = ["armed", "listening", "speaking", "settling"].includes(liveView.status);
  const micLabelId = useId();
  const modeLabelId = useId();
  const [hotkeyDraft, setHotkeyDraft] = useState(liveView.hotkey);

  useEffect(() => {
    setHotkeyDraft(liveView.hotkey);
  }, [liveView.hotkey]);

  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent className="w-[min(420px,calc(100vw-24px))] sm:max-w-md" side="right">
        <SheetHeader>
          <SheetTitle>Settings</SheetTitle>
          <SheetDescription>Status and controls.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-6 px-4">
          <div className="flex flex-col gap-3">
            <StatusRow icon={BadgeCheck} label="Status" value={status} />
            <StatusRow icon={Server} label="Server" value={serverLabel} />
            <StatusRow icon={Sparkles} label="Local fallback" value={setupLabel} />
            <StatusRow icon={Sparkles} label="Fallback files" value={engineBinaryStatus} wrap />
            <StatusRow icon={Sparkles} label="Fallback model" value={model} wrap />
            <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
            <StatusRow icon={FolderOutput} label="Output" value="Source folder" />
          </div>
          <div className="rounded-md border bg-muted/20 p-3">
            <div className="mb-2 flex items-center gap-2 text-sm font-medium">
              <Mic />
              Live
            </div>
            <div className="mb-3 grid gap-2 text-sm">
              <StatusRow icon={Mic} label="State" value={liveStatusLabel(liveView.status)} />
              <StatusRow icon={Server} label="Route" value={liveRouteLabel(liveView.route)} />
              <StatusRow icon={Keyboard} label="Shortcut" value={liveView.hotkey || "Off"} wrap />
            </div>
            <div className="grid gap-3">
              <label className="grid gap-1 text-xs font-medium">
                Shortcut
                <Input
                  disabled={liveBusy || liveActive}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") onSetLiveHotkey(hotkeyDraft);
                  }}
                  placeholder="Ctrl+Shift+Space"
                  value={hotkeyDraft}
                  onChange={(event) => setHotkeyDraft(event.currentTarget.value)}
                />
              </label>
              <div className="grid gap-1.5">
                <Label className="text-xs" id={modeLabelId}>
                  Mode
                </Label>
                <ToggleGroup
                  aria-labelledby={modeLabelId}
                  className="grid grid-cols-2"
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
              </div>
              <div className="grid gap-1.5">
                <Label className="text-xs" id={micLabelId}>
                  Microphone
                </Label>
                <Select
                  disabled={liveBusy || liveActive}
                  onValueChange={(value) => onSetInputDevice(value === "default" ? undefined : value)}
                  value={liveView.inputDeviceId ?? "default"}
                >
                  <SelectTrigger aria-labelledby={micLabelId} className="w-full">
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
              </div>
              {liveSettingsError || liveView.error ? (
                <p className="text-xs text-destructive">{liveSettingsError || liveView.error}</p>
              ) : null}
              <div className="flex flex-wrap gap-2">
                <Button disabled={liveBusy || liveActive} onClick={() => onSetLiveOverlayEnabled(liveView.visibility === "hidden")} size="sm" type="button" variant="outline">
                  <Mic data-icon="inline-start" />
                  {liveView.visibility === "hidden" ? "Show overlay" : "Hide overlay"}
                </Button>
                <Button disabled={liveBusy} onClick={liveActive ? onStopLive : onStartLive} size="sm" type="button">
                  <Mic data-icon="inline-start" />
                  {liveActive ? "Stop" : "Start"}
                </Button>
                <Button disabled={liveBusy || liveActive} onClick={onPreflightLiveInput} size="sm" type="button" variant="outline">
                  Check mic
                </Button>
                <Button disabled={liveBusy || liveActive || hotkeyDraft === liveView.hotkey} onClick={() => onSetLiveHotkey(hotkeyDraft)} size="sm" type="button" variant="outline">
                  Apply
                </Button>
                <Button disabled={liveBusy || liveActive} onClick={onResetLiveHotkey} size="sm" type="button" variant="ghost">
                  Reset
                </Button>
                <Button disabled={liveBusy || liveActive || !liveView.hotkey} onClick={onClearLiveHotkey} size="sm" type="button" variant="ghost">
                  Clear
                </Button>
              </div>
            </div>
          </div>
          <div className="rounded-md border bg-muted/20 p-3">
            <div className="mb-2 flex items-center gap-2 text-sm font-medium">
              <Sparkles />
              Fallback
            </div>
            <p className="mb-3 break-words text-xs leading-5 text-muted-foreground">
              Moonshine tiny and punctuation files install to {setupRoot || "app data"}.
            </p>
            <div className="flex flex-wrap gap-2">
              <Button disabled={busy || liveActive} onClick={onInstallFallback} size="sm" type="button">
                <Sparkles data-icon="inline-start" />
                {busy ? "Working" : engineReady ? "Reinstall" : "Install"}
              </Button>
              <Button
                disabled={busy || liveActive || !canRemove}
                onClick={onRemoveFallback}
                size="sm"
                type="button"
                variant="outline"
              >
                <Trash2 data-icon="inline-start" />
                Remove files
              </Button>
              <Button
                disabled={busy || liveActive}
                onClick={() => onSetFallbackEnabled(!fallbackEnabled)}
                size="sm"
                type="button"
                variant="outline"
              >
                <Server data-icon="inline-start" />
                {fallbackEnabled ? "Disable" : "Enable"}
              </Button>
              {!engineReady ? (
                <Button disabled={busy} onClick={onSkipSetup} size="sm" type="button" variant="ghost">
                  Skip
                </Button>
              ) : null}
            </div>
          </div>
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
