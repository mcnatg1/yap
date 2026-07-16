import { Copy } from "@phosphor-icons/react/Copy";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import { CloudArrowUp as UploadCloud } from "@phosphor-icons/react/CloudArrowUp";

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

export function HelpSheet({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  return (
    <Sheet onOpenChange={onOpenChange} open={open}>
      <SheetContent className="w-[min(420px,calc(100vw-24px))] sm:max-w-md" side="right">
        <SheetHeader>
          <SheetTitle>Help</SheetTitle>
          <SheetDescription>Core controls.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-3 px-4">
          <StatusRow
            icon={UploadCloud}
            label="Add files"
            value="Choose files or drag recordings onto Transcribe. They wait in the organization server queue."
            wrap
          />
          <StatusRow
            icon={Sparkles}
            label="Transcribe"
            value="Saves beside the source when allowed, otherwise to local Yap transcripts."
            wrap
          />
          <StatusRow
            icon={Copy}
            label="Copy"
            value="Copies transcript text after a file finishes."
            wrap
          />
          <StatusRow
            icon={FolderOpen}
            label="Reveal"
            value="Shows the saved transcript in File Explorer."
            wrap
          />
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
