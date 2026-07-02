import { BadgeCheck, Copy, FolderOpen, FolderOutput, LockKeyhole, Server, Sparkles, UploadCloud } from "lucide-react";

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
  engineBinaryStatus,
  model,
  onOpenChange,
  open,
  status,
}: {
  auth: string;
  engineBinaryStatus: string;
  model: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  status: string;
}) {
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
