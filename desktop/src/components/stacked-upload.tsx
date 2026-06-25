import { AnimatePresence, motion } from "framer-motion";
import type { KeyboardEvent } from "react";
import {
  CheckCircle2,
  Clock3,
  FileAudio,
  FolderOpen,
  Loader2,
  Trash2,
  XCircle,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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

export function StackedUpload({ items, onRemove, onReveal, onSelect, selectedId }: Props) {
  if (!items.length) {
    return (
      <div className="grid min-h-[200px] place-items-center rounded-lg border border-dashed bg-muted">
        <div className="flex flex-col items-center gap-3 text-center">
          <div className="grid size-12 place-items-center rounded-lg border bg-card text-muted-foreground">
            <FileAudio className="size-5" />
          </div>
          <div>
            <h3 className="text-sm font-semibold">No audio queued</h3>
            <p className="mt-1 text-xs text-muted-foreground">Drop files to begin.</p>
          </div>
        </div>
      </div>
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

  function selectFromKeyboard(event: KeyboardEvent<HTMLLIElement>) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect(item.id);
    }
  }

  return (
    <motion.li
      animate={{ opacity: 1, y: 0, scale: 1 }}
      className={cn(
        "relative list-none overflow-hidden rounded-lg border bg-card p-3 outline-none transition-[border-color,box-shadow,background-color]",
        "focus-visible:ring-2 focus-visible:ring-ring/50",
        isSelected && "border-primary ring-2 ring-primary/15",
        offset > 0 && "shadow-sm",
      )}
      exit={{ opacity: 0, x: 12, scale: 0.98 }}
      initial={{ opacity: 0, y: 16, scale: 0.98 }}
      layout
      onClick={() => onSelect(item.id)}
      onKeyDown={selectFromKeyboard}
      role="button"
      tabIndex={0}
      transition={{ duration: 0.18, ease: "easeOut" }}
    >
      <div className="flex items-center gap-3">
        <div className="grid size-10 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
          <FileAudio className="size-5" />
        </div>

        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold">{item.name}</div>
          <div className="mt-0.5 truncate text-xs text-muted-foreground">{detail}</div>
        </div>

        <Badge variant={meta.variant}>
          <Icon className={cn(item.status === "running" && "animate-spin")} />
          {meta.label}
        </Badge>

        {item.output ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
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
              </Button>
            </TooltipTrigger>
            <TooltipContent>Reveal transcript</TooltipContent>
          </Tooltip>
        ) : (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label="Remove file"
                disabled={item.status === "running"}
                onClick={(event) => {
                  event.stopPropagation();
                  onRemove(item.id);
                }}
                size="icon-sm"
                type="button"
                variant="ghost"
              >
                <Trash2 />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Remove file</TooltipContent>
          </Tooltip>
        )}
      </div>
      <Progress className="mt-3 h-1.5" value={meta.progress} />
    </motion.li>
  );
}
