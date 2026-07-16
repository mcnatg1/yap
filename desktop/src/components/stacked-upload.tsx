import { CheckCircle as CheckCircle2 } from "@phosphor-icons/react/CheckCircle";
import { Clock as Clock3 } from "@phosphor-icons/react/Clock";
import { FileAudio } from "@phosphor-icons/react/FileAudio";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { SpinnerGap as Loader2 } from "@phosphor-icons/react/SpinnerGap";
import { Trash as Trash2 } from "@phosphor-icons/react/Trash";
import { XCircle } from "@phosphor-icons/react/XCircle";

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
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  isRecordingActive,
  isRecordingCancellable,
  queuedServerMessage,
  type RecordingJobStatus,
  type RecordingJobView,
} from "@/lib/recording-job";
import { cn } from "@/lib/utils";

type Props = {
  items: RecordingJobView[];
  onRemove: (id: string) => void;
  onReveal: (path: string) => void;
  onSelect: (id: string) => void;
  selectedId?: string;
};

const statusMeta = {
  accepted: {
    label: "Ready",
    icon: Clock3,
    variant: "secondary" as const,
  },
  preflighting: {
    label: "Checking",
    icon: Loader2,
    variant: "outline" as const,
  },
  blocked_setup_required: {
    label: "Setup",
    icon: XCircle,
    variant: "secondary" as const,
  },
  blocked_server_unavailable: {
    label: "Server",
    icon: XCircle,
    variant: "secondary" as const,
  },
  blocked_sign_in_required: {
    label: "Sign in",
    icon: XCircle,
    variant: "secondary" as const,
  },
  queued_server: {
    label: "Server queued",
    icon: Clock3,
    variant: "secondary" as const,
  },
  queued_local_fallback: {
    label: "Local queued",
    icon: Clock3,
    variant: "secondary" as const,
  },
  preprocessing: {
    label: "Preparing",
    icon: Loader2,
    variant: "outline" as const,
  },
  uploading: {
    label: "Uploading",
    icon: Loader2,
    variant: "outline" as const,
  },
  server_processing: {
    label: "Server",
    icon: Loader2,
    variant: "outline" as const,
  },
  local_transcribing: {
    label: "Local",
    icon: Loader2,
    variant: "outline" as const,
  },
  saving: {
    label: "Saving",
    icon: Loader2,
    variant: "outline" as const,
  },
  diarization_queued: {
    label: "Speakers queued",
    icon: Clock3,
    variant: "secondary" as const,
  },
  diarization_running: {
    label: "Speakers",
    icon: Loader2,
    variant: "outline" as const,
  },
  complete: {
    label: "Done",
    icon: CheckCircle2,
    variant: "default" as const,
  },
  partial: {
    label: "Partial",
    icon: CheckCircle2,
    variant: "secondary" as const,
  },
  failed: {
    label: "Error",
    icon: XCircle,
    variant: "destructive" as const,
  },
  cancelled: {
    label: "Cancelled",
    icon: XCircle,
    variant: "secondary" as const,
  },
};

const attachmentState = {
  accepted: "idle",
  preflighting: "processing",
  blocked_setup_required: "idle",
  blocked_server_unavailable: "idle",
  blocked_sign_in_required: "idle",
  queued_server: "idle",
  queued_local_fallback: "idle",
  preprocessing: "processing",
  uploading: "uploading",
  server_processing: "processing",
  local_transcribing: "processing",
  saving: "processing",
  diarization_queued: "idle",
  diarization_running: "processing",
  complete: "done",
  partial: "done",
  failed: "error",
  cancelled: "idle",
} as const satisfies Record<RecordingJobStatus, "idle" | "uploading" | "processing" | "error" | "done">;

export function StackedUpload({ items, onRemove, onReveal, onSelect, selectedId }: Props) {
  if (!items.length) {
    return (
      <Empty>
        <EmptyMedia>
          <FileAudio />
        </EmptyMedia>
        <div>
          <EmptyTitle>No audio queued</EmptyTitle>
          <EmptyDescription>No files are waiting for the organization server.</EmptyDescription>
        </div>
      </Empty>
    );
  }

  return (
    <ScrollArea className="h-[260px] pr-3">
      <ul className="flex flex-col gap-2">
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
  item: RecordingJobView;
  offset: number;
  onRemove: (id: string) => void;
  onReveal: (path: string) => void;
  onSelect: (id: string) => void;
}) {
  const meta = statusMeta[item.status];
  const Icon = meta.icon;
  const isActive = isRecordingActive(item.status);
  const isCancellable = isRecordingCancellable(item.status);
  const canRemove = item.status === "failed" || isCancellable;
  const removeLabel = isActive && isCancellable ? "Cancel recording" : "Remove file";
  const detail = item.status === "complete"
      ? "Saved"
      : item.status === "partial"
        ? "Partial"
      : item.status === "cancelled"
        ? "Cancelled"
      : isActive
        ? meta.label
        : item.status === "accepted"
          ? "Ready"
          : item.status === "queued_server"
            ? queuedServerMessage
          : item.status.startsWith("blocked_")
            ? "Waiting"
          : "Needs attention";

  return (
    <li className="list-none">
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
              className={cn(isActive && "animate-spin motion-reduce:animate-none")}
              data-icon="inline-start"
            />
            {meta.label}
          </Badge>

          {item.outputPath ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <AttachmentAction
                  aria-label="Reveal transcript"
                  onClick={(event) => {
                    event.stopPropagation();
                    onReveal(item.outputPath!);
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
                  aria-label={removeLabel}
                  disabled={!canRemove}
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
              <TooltipContent>{removeLabel}</TooltipContent>
            </Tooltip>
          )}
        </AttachmentActions>

        <AttachmentTrigger aria-label={`Select ${item.name}`} onClick={() => onSelect(item.id)} />
      </Attachment>
    </li>
  );
}
