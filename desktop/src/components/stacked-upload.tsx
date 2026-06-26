import { AnimatePresence, motion } from "framer-motion";
import {
  CheckCircle2,
  Clock3,
  FileAudio,
  FolderOpen,
  Loader2,
  Trash2,
  XCircle,
} from "lucide-react";

import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
} from "@/components/ui/attachment";
import { Badge } from "@/components/ui/badge";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Progress } from "@/components/ui/progress";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export type UploadStatus = "queued" | "running" | "done" | "error";

export type UploadItem = {
  id: number;
  path: string;
  name: string;
  status: UploadStatus;
  output?: string;
  error?: string;
};

type Props = {
  items: UploadItem[];
  onRemove: (id: number) => void;
  onReveal: (path: string) => void;
  onSelect: (id: number) => void;
  selectedId?: number;
};

const statusMeta = {
  queued: {
    label: "Queued",
    icon: Clock3,
    progress: 16,
    variant: "secondary" as const,
  },
  running: {
    label: "Running",
    icon: Loader2,
    progress: 62,
    variant: "outline" as const,
  },
  done: {
    label: "Done",
    icon: CheckCircle2,
    progress: 100,
    variant: "default" as const,
  },
  error: {
    label: "Error",
    icon: XCircle,
    progress: 100,
    variant: "destructive" as const,
  },
};

const attachmentState = {
  queued: "idle",
  running: "processing",
  done: "done",
  error: "error",
} as const satisfies Record<UploadStatus, "idle" | "uploading" | "processing" | "error" | "done">;

export function StackedUpload({ items, onRemove, onReveal, onSelect, selectedId }: Props) {
  if (!items.length) {
    return (
      <Empty>
        <EmptyMedia>
          <FileAudio />
        </EmptyMedia>
        <div>
          <EmptyTitle>No audio queued</EmptyTitle>
          <EmptyDescription>Drop files to begin.</EmptyDescription>
        </div>
      </Empty>
    );
  }

  return (
    <ScrollArea className="h-[260px] pr-3">
      <ul className="flex flex-col gap-2">
        <AnimatePresence initial={false}>
          {items.map((item, index) => (
            <UploadCard
              isSelected={selectedId === item.id}
              item={item}
              key={item.id}
              offset={index}
              onRemove={onRemove}
              onReveal={onReveal}
              onSelect={onSelect}
            />
          ))}
        </AnimatePresence>
      </ul>
    </ScrollArea>
  );
}

function UploadCard({
  isSelected,
  item,
  offset,
  onRemove,
  onReveal,
  onSelect,
}: {
  isSelected: boolean;
  item: UploadItem;
  offset: number;
  onRemove: (id: number) => void;
  onReveal: (path: string) => void;
  onSelect: (id: number) => void;
}) {
  const meta = statusMeta[item.status];
  const Icon = meta.icon;
  const detail = item.error ?? item.output ?? item.path;

  return (
    <motion.li
      animate={{ opacity: 1, y: 0, scale: 1 }}
      className="list-none"
      exit={{ opacity: 0, x: 12, scale: 0.98 }}
      initial={{ opacity: 0, y: 16, scale: 0.98 }}
      layout
      transition={{ duration: 0.18, ease: "easeOut" }}
    >
      <Attachment
        className={cn(
          "w-full cursor-pointer overflow-hidden rounded-lg outline-none transition-[border-color,box-shadow,background-color]",
          "focus-visible:ring-2 focus-visible:ring-ring/50",
          isSelected && "border-primary ring-2 ring-primary/15",
          offset > 0 && "shadow-sm",
        )}
        state={attachmentState[item.status]}
      >
        <AttachmentMedia>
          <FileAudio />
        </AttachmentMedia>

        <AttachmentContent>
          <AttachmentTitle>{item.name}</AttachmentTitle>
          <AttachmentDescription>{detail}</AttachmentDescription>
        </AttachmentContent>

        <AttachmentActions className="gap-2">
          <Badge variant={meta.variant}>
            <Icon className={cn(item.status === "running" && "animate-spin")} data-icon="inline-start" />
            {meta.label}
          </Badge>

          {item.output ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <AttachmentAction
                  aria-label="Reveal transcript"
                  onClick={(event) => {
                    event.stopPropagation();
                    onReveal(item.output!);
                  }}
                  size="icon-sm"
                  type="button"
                  variant="outline"
                >
                  <FolderOpen />
                </AttachmentAction>
              </TooltipTrigger>
              <TooltipContent>Reveal transcript</TooltipContent>
            </Tooltip>
          ) : (
            <Tooltip>
              <TooltipTrigger asChild>
                <AttachmentAction
                  aria-label="Remove file"
                  disabled={item.status === "running"}
                  onClick={(event) => {
                    event.stopPropagation();
                    onRemove(item.id);
                  }}
                  size="icon-sm"
                  type="button"
                >
                  <Trash2 />
                </AttachmentAction>
              </TooltipTrigger>
              <TooltipContent>Remove file</TooltipContent>
            </Tooltip>
          )}
        </AttachmentActions>

        <AttachmentTrigger aria-label={`Select ${item.name}`} onClick={() => onSelect(item.id)} />
      </Attachment>
      <Progress className="mt-3 h-1.5" value={meta.progress} />
    </motion.li>
  );
}
