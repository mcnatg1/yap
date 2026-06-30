import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import {
  CheckCircle2,
  Clock3,
  FileAudio,
  FolderOpen,
  Loader2,
  RotateCcw,
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
import { formatElapsed } from "@/lib/app-types";

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
  elapsedSeconds?: number;
  items: UploadItem[];
  onRemove: (id: number) => void;
  onRetry: (id: number) => void;
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
    progress: null,
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

export function StackedUpload({ elapsedSeconds, items, onRemove, onRetry, onReveal, onSelect, selectedId }: Props) {
  const reducedMotion = useReducedMotion() ?? false;

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
              elapsedSeconds={item.status === "running" ? elapsedSeconds : undefined}
              isSelected={selectedId === item.id}
              item={item}
              key={item.id}
              offset={index}
              onRemove={onRemove}
              onRetry={onRetry}
              onReveal={onReveal}
              onSelect={onSelect}
              reducedMotion={reducedMotion}
            />
          ))}
        </AnimatePresence>
      </ul>
    </ScrollArea>
  );
}

function UploadCard({
  elapsedSeconds,
  isSelected,
  item,
  offset,
  onRemove,
  onRetry,
  onReveal,
  onSelect,
  reducedMotion,
}: {
  elapsedSeconds?: number;
  isSelected: boolean;
  item: UploadItem;
  offset: number;
  onRemove: (id: number) => void;
  onRetry: (id: number) => void;
  onReveal: (path: string) => void;
  onSelect: (id: number) => void;
  reducedMotion: boolean;
}) {
  const meta = statusMeta[item.status];
  const Icon = meta.icon;
  const detail =
    item.error ??
    (item.status === "done"
      ? "Transcript saved"
      : item.status === "running"
        ? elapsedSeconds
          ? `Transcribing locally · ${formatElapsed(elapsedSeconds)}`
          : "Transcribing locally…"
        : item.status === "queued"
          ? "Ready to transcribe"
          : "Needs attention");

  const cardTransition = reducedMotion
    ? { duration: 0 }
    : { delay: offset * 0.1, duration: 0.18, ease: "easeOut" as const };

  return (
    <motion.li
      animate={reducedMotion ? { opacity: 1 } : { opacity: 1, y: 0, scale: 1 }}
      className="list-none"
      exit={reducedMotion ? { opacity: 0 } : { opacity: 0, y: 4, scale: 0.99 }}
      initial={reducedMotion ? false : { opacity: 0, y: 8, scale: 0.98 }}
      layout={!reducedMotion}
      transition={cardTransition}
    >
      <Attachment
        className={cn(
          "w-full cursor-pointer overflow-hidden outline-none transition-[border-color,box-shadow,background-color]",
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
            <Icon
              className={cn(item.status === "running" && "animate-spin motion-reduce:animate-none")}
              data-icon="inline-start"
            />
            {item.status === "running" && elapsedSeconds !== undefined ? (
              <span className="tabular-nums">{formatElapsed(elapsedSeconds)}</span>
            ) : (
              meta.label
            )}
          </Badge>

          {item.status === "error" ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <AttachmentAction
                  aria-label="Retry transcription"
                  onClick={(event) => {
                    event.stopPropagation();
                    onRetry(item.id);
                  }}
                  size="icon-sm"
                  type="button"
                  variant="outline"
                >
                  <RotateCcw />
                </AttachmentAction>
              </TooltipTrigger>
              <TooltipContent>Retry</TooltipContent>
            </Tooltip>
          ) : null}

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
      {meta.progress === null ? (
        <div aria-hidden className="mt-3 h-1.5 overflow-hidden rounded-full bg-primary/20">
          <div className="h-full w-1/3 animate-pulse motion-reduce:animate-none rounded-full bg-primary" />
        </div>
      ) : (
        <Progress className="mt-3 h-1.5" value={meta.progress} />
      )}
    </motion.li>
  );
}
