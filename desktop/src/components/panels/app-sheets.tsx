import {
  BadgeCheck,
  Copy,
  FolderOpen,
  FolderOutput,
  LockKeyhole,
  Keyboard,
  Mic,
  Server,
  Sparkles,
  Trash2,
  UploadCloud,
} from "lucide-react";
import { useEffect, useState } from "react";

import { StatusRow } from "@/components/app/status-row";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
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
  const [hotkeyDraft, setHotkeyDraft] = useState(liveView.hotkey);

  useEffect(() => {
    setHotkeyDraft(liveView.hotkey);
  }, [liveView.hotkey]);

  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent className="w-[min(420px,calc(100vw-24px))] sm:max-w-md" side="right">
        <SheetHeader>
          <SheetTitle>Settings</SheetTitle>
          <SheetDescription>Runtime status.</SheetDescription>
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
                  disabled={liveBusy}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") onSetLiveHotkey(hotkeyDraft);
                  }}
                  placeholder="Ctrl+Shift+Space"
                  value={hotkeyDraft}
                  onChange={(event) => setHotkeyDraft(event.currentTarget.value)}
                />
              </label>
              <label className="grid gap-1 text-xs font-medium">
                Mode
                <select
                  className="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
                  disabled={liveBusy}
                  onChange={(event) => onSetLiveCaptureMode(event.currentTarget.value as LiveCaptureMode)}
                  value={liveView.captureMode}
                >
                  <option value="pushToTalk">Push to talk</option>
                  <option value="toggle">Toggle</option>
                </select>
              </label>
              <label className="grid gap-1 text-xs font-medium">
                Microphone
                <select
                  className="h-9 rounded-md border border-input bg-transparent px-3 text-sm"
                  disabled={liveBusy}
                  onChange={(event) => onSetInputDevice(event.currentTarget.value || undefined)}
                  value={liveView.inputDeviceId ?? ""}
                >
                  <option value="">System default</option>
                  {liveInputDevices.map((device) => (
                    <option key={device.id} value={device.id}>
                      {device.label}{device.isDefault ? " (default)" : ""}
                    </option>
                  ))}
                </select>
              </label>
              {liveSettingsError || liveView.error ? (
                <p className="text-xs text-destructive">{liveSettingsError || liveView.error}</p>
              ) : null}
              <div className="flex flex-wrap gap-2">
                <Button disabled={liveBusy} onClick={() => onSetLiveOverlayEnabled(liveView.visibility === "hidden")} size="sm" type="button" variant="outline">
                  <Mic />
                  {liveView.visibility === "hidden" ? "Show overlay" : "Hide overlay"}
                </Button>
                <Button disabled={liveBusy} onClick={liveActive ? onStopLive : onStartLive} size="sm" type="button">
                  <Mic />
                  {liveActive ? "Stop" : "Start"}
                </Button>
                <Button disabled={liveBusy} onClick={onPreflightLiveInput} size="sm" type="button" variant="outline">
                  Check mic
                </Button>
                <Button disabled={liveBusy || hotkeyDraft === liveView.hotkey} onClick={() => onSetLiveHotkey(hotkeyDraft)} size="sm" type="button" variant="outline">
                  Apply
                </Button>
                <Button disabled={liveBusy} onClick={onResetLiveHotkey} size="sm" type="button" variant="ghost">
                  Reset
                </Button>
                <Button disabled={liveBusy || !liveView.hotkey} onClick={onClearLiveHotkey} size="sm" type="button" variant="ghost">
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
              Installs verified CrispASR, Moonshine tiny, and punctuation files in {setupRoot || "app data"}.
            </p>
            <div className="flex flex-wrap gap-2">
              <Button disabled={busy} onClick={onInstallFallback} size="sm" type="button">
                <Sparkles />
                {busy ? "Working" : engineReady ? "Reinstall" : "Install"}
              </Button>
              <Button
                disabled={busy || !canRemove}
                onClick={onRemoveFallback}
                size="sm"
                type="button"
                variant="outline"
              >
                <Trash2 />
                Remove files
              </Button>
              <Button
                disabled={busy}
                onClick={() => onSetFallbackEnabled(!fallbackEnabled)}
                size="sm"
                type="button"
                variant="outline"
              >
                <Server />
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
          <SheetDescription>Quick map of the working controls.</SheetDescription>
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
