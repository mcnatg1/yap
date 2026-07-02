import {
  BadgeCheck,
  Copy,
  FolderOpen,
  FolderOutput,
  LockKeyhole,
  Server,
  Sparkles,
  Trash2,
  UploadCloud,
} from "lucide-react";

import { StatusRow } from "@/components/app/status-row";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";

export function SettingsSheet({
  auth,
  busy,
  engineReady,
  engineBinaryStatus,
  fallbackEnabled,
  model,
  modelInstalled,
  onOpenChange,
  onInstallFallback,
  onRemoveFallback,
  onSetFallbackEnabled,
  onSkipSetup,
  open,
  status,
}: {
  auth: string;
  busy: boolean;
  engineReady: boolean;
  engineBinaryStatus: string;
  fallbackEnabled: boolean;
  model: string;
  modelInstalled: boolean;
  onOpenChange: (open: boolean) => void;
  onInstallFallback: () => void;
  onRemoveFallback: () => void;
  onSetFallbackEnabled: (enabled: boolean) => void;
  onSkipSetup: () => void;
  open: boolean;
  status: string;
}) {
  const canRemove = modelInstalled || engineBinaryStatus === "Installed";

  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent className="w-[min(420px,calc(100vw-24px))] sm:max-w-md" side="right">
        <SheetHeader>
          <SheetTitle>Settings</SheetTitle>
          <SheetDescription>Connection and fallback status.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-6 px-4">
          <div className="flex flex-col gap-3">
            <StatusRow icon={BadgeCheck} label="Status" value={status} />
            <StatusRow icon={Server} label="Server" value="Not connected" />
            <StatusRow icon={Sparkles} label="Local fallback" value={engineBinaryStatus} wrap />
            <StatusRow icon={Sparkles} label="Fallback model" value={model} wrap />
            <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
            <StatusRow icon={FolderOutput} label="Output" value="Source folder" />
          </div>
          <div className="rounded-md border bg-muted/20 p-3">
            <div className="mb-2 flex items-center gap-2 text-sm font-medium">
              <Sparkles />
              Local fallback setup
            </div>
            <p className="mb-3 text-sm text-muted-foreground">
              Moonshine tiny downloads only when you install it here.
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
