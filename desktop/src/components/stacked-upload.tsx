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
};

const statusMeta = {
  queued: {
    label: "Queued",
    icon: Clock3,
    className: "bg-slate-100 text-slate-600 ring-slate-200",
    progress: "w-1/4 bg-slate-300",
  },
  running: {
    label: "Running",
    icon: Loader2,
    className: "bg-amber-50 text-amber-700 ring-amber-200",
    progress: "w-2/3 bg-amber-300",
  },
  done: {
    label: "Done",
    icon: CheckCircle2,
    className: "bg-teal-50 text-teal-700 ring-teal-200",
    progress: "w-full bg-teal-300",
  },
  error: {
    label: "Error",
    icon: XCircle,
    className: "bg-red-50 text-red-700 ring-red-200",
    progress: "w-full bg-red-300",
  },
};

export function StackedUpload({ items, onRemove, onReveal }: Props) {
  if (!items.length) {
    return (
      <div className="grid min-h-[190px] place-items-center rounded-lg border border-dashed border-slate-200 bg-slate-50/70">
        <div className="text-center">
          <div className="mx-auto grid size-12 place-items-center rounded-lg border border-slate-200 bg-white text-slate-500 shadow-sm">
            <FileAudio className="size-5" />
          </div>
          <h3 className="mt-4 text-sm font-semibold text-slate-900">No audio queued</h3>
          <p className="mt-1 text-xs text-slate-500">Files stay on this machine.</p>
        </div>
      </div>
    );
  }

  return (
    <ul className="relative min-h-[190px] space-y-2 overflow-auto pr-1">
      <AnimatePresence initial={false}>
        {items.map((item, index) => (
          <UploadCard
            key={item.id}
            item={item}
            offset={index}
            onRemove={onRemove}
            onReveal={onReveal}
          />
        ))}
      </AnimatePresence>
    </ul>
  );
}

function UploadCard({
  item,
  offset,
  onRemove,
  onReveal,
}: {
  item: UploadItem;
  offset: number;
  onRemove: (id: number) => void;
  onReveal: (path: string) => void;
}) {
  const meta = statusMeta[item.status];
  const Icon = meta.icon;
  const detail = item.error ?? item.output ?? item.path;

  return (
    <motion.li
      layout
      initial={{ opacity: 0, y: 16, scale: 0.98 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, x: 12, scale: 0.98 }}
      transition={{ duration: 0.18, ease: "easeOut" }}
      className={cn(
        "group relative overflow-hidden rounded-lg border border-slate-200 bg-white p-3 shadow-sm",
        offset > 0 && "shadow-[0_8px_24px_rgba(15,23,42,0.06)]",
      )}
    >
      <div className={cn("absolute inset-x-0 bottom-0 h-0.5 transition-all", meta.progress)} />
      <div className="flex items-center gap-3">
        <div className="grid size-10 shrink-0 place-items-center rounded-lg bg-slate-100 text-slate-600">
          <FileAudio className="size-5" />
        </div>

        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-slate-950">{item.name}</div>
          <div className="mt-0.5 truncate text-xs text-slate-500">{detail}</div>
        </div>

        <div
          className={cn(
            "inline-flex min-w-24 items-center justify-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-semibold ring-1",
            meta.className,
          )}
        >
          <Icon className={cn("size-3.5", item.status === "running" && "animate-spin")} />
          {meta.label}
        </div>

        {item.output ? (
          <button
            className="grid size-8 place-items-center rounded-md border border-slate-200 text-slate-500 hover:border-teal-200 hover:text-teal-700"
            onClick={() => onReveal(item.output!)}
            title="Reveal transcript"
            type="button"
          >
            <FolderOpen className="size-4" />
          </button>
        ) : (
          <button
            className="grid size-8 place-items-center rounded-md border border-slate-200 text-slate-400 hover:border-red-200 hover:text-red-600"
            onClick={() => onRemove(item.id)}
            title="Remove file"
            type="button"
          >
            <Trash2 className="size-4" />
          </button>
        )}
      </div>
    </motion.li>
  );
}
