import { type UploadItem } from "@/components/stacked-upload";
import { TranscriptPanel } from "@/components/panels/transcript-panel";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function TranscriptReviewDialog({
  elapsedSeconds,
  item,
  onCopy,
  onOpen,
  onOpenChange,
  onOpenHelp,
  onRetry,
  onReveal,
  open,
  running,
  text,
}: {
  elapsedSeconds: number;
  item?: UploadItem;
  onCopy: (item: UploadItem) => void;
  onOpen: (path: string) => void;
  onOpenChange: (open: boolean) => void;
  onOpenHelp?: () => void;
  onRetry: (id: number) => void;
  onReveal: (path: string) => void;
  open: boolean;
  running: boolean;
  text?: string;
}) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="h-[min(82vh,720px)] max-w-none gap-0 overflow-hidden rounded-[28px] border-0 bg-card/90 p-2 shadow-[0_0_0_1px_rgba(255,255,255,0.56),0_24px_80px_rgba(0,0,0,0.18)] backdrop-blur-2xl duration-150 data-[state=closed]:slide-out-to-bottom-1 data-[state=open]:slide-in-from-bottom-2 motion-reduce:animate-none sm:w-[min(900px,calc(100vw-2rem))] sm:max-w-none"
        showCloseButton
      >
        <DialogHeader className="sr-only">
          <DialogTitle>{item?.name ?? "Recording review"}</DialogTitle>
          <DialogDescription>Recording playback and transcript review.</DialogDescription>
        </DialogHeader>
        {item ? (
          <TranscriptPanel
            className="h-full min-h-0 rounded-[22px] bg-card/95 shadow-none xl:static xl:top-auto xl:min-h-0 [&_[data-slot=card-header]]:pr-14"
            elapsedSeconds={elapsedSeconds}
            item={item}
            onCopy={onCopy}
            onOpen={onOpen}
            onOpenHelp={onOpenHelp}
            onRetry={onRetry}
            onReveal={onReveal}
            running={running}
            text={text}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
