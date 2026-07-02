import { BadgeCheck, Copy, Cpu, Download, FolderOpen, FolderOutput, LockKeyhole, Sparkles, UploadCloud } from "lucide-react";

import { StatusRow } from "@/components/app/status-row";
import { Button } from "@/components/ui/button";
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldSeparator,
} from "@/components/ui/field";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import type { GpuSetting } from "@/settings";

export function SettingsSheet({
  auth,
  engineBinaryStatus,
  engineInstalling,
  gpuAdapter,
  gpuAvailable,
  model,
  modelInstalled,
  onInstallEngine,
  onOpenChange,
  onUseGpuChange,
  open,
  runner,
  saving,
  status,
  useGpu,
}: {
  auth: string;
  engineBinaryStatus: string;
  engineInstalling: boolean;
  gpuAdapter: string;
  gpuAvailable: boolean;
  model: string;
  modelInstalled: boolean;
  onInstallEngine: () => void;
  onOpenChange: (open: boolean) => void;
  onUseGpuChange: (useGpu: GpuSetting) => void;
  open: boolean;
  runner: string;
  saving: boolean;
  status: string;
  useGpu: GpuSetting;
}) {
  const gpuEnabled = useGpu === "auto";
  const engineNeedsInstall =
    engineBinaryStatus.includes("Not installed") || engineBinaryStatus.includes("Invalid");

  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent className="w-[min(420px,calc(100vw-24px))] sm:max-w-md" side="right">
        <SheetHeader>
          <SheetTitle>Settings</SheetTitle>
          <SheetDescription>Transcription runner and local output.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-6 px-4">
          <FieldGroup>
            <Field orientation="horizontal">
              <FieldContent>
                <FieldLabel htmlFor="settings-use-gpu">Use GPU when available</FieldLabel>
                <FieldDescription>
                  {gpuAvailable
                    ? "Offloads transcription and polish to your NVIDIA GPU. CPU is the default."
                    : "No NVIDIA GPU detected. Transcription stays on CPU."}
                </FieldDescription>
              </FieldContent>
              <Switch
                checked={gpuEnabled}
                disabled={!gpuAvailable || saving}
                id="settings-use-gpu"
                onCheckedChange={(checked) => onUseGpuChange(checked ? "auto" : "cpu")}
              />
            </Field>
          </FieldGroup>

          <FieldSeparator>Status</FieldSeparator>

          <div className="flex flex-col gap-3">
            <StatusRow icon={BadgeCheck} label="Status" value={status} />
            <StatusRow icon={Cpu} label="Engine" value={engineBinaryStatus} wrap />
            {engineNeedsInstall ? (
              <Button disabled={engineInstalling} onClick={onInstallEngine} type="button" variant="secondary">
                <Download data-icon="inline-start" />
                {engineInstalling ? "Installing engine…" : "Install transcription engine"}
              </Button>
            ) : null}
            <StatusRow
              icon={Sparkles}
              label="Model"
              value={modelInstalled ? model : `${model} (re-run installer)`}
            />
            <StatusRow icon={Cpu} label="Runner" value={runner} />
            {gpuAdapter ? <StatusRow icon={Cpu} label="GPU" value={gpuAdapter} /> : null}
            <StatusRow icon={LockKeyhole} label="Auth" value={auth} />
            <StatusRow icon={FolderOutput} label="Output" value="Source folder, local fallback" />
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
