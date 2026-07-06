import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import type { TranscriptHistoryEntry } from "@/history";
import { formatHistoryDate } from "@/lib/app-types";
import { cn } from "@/lib/utils";

const OPEN_DELAY_MS = 350;
const CLOSE_DELAY_MS = 80;

export function HistoryEntryPreview({
  entry,
  onLoadPreviewText,
  onReview,
}: {
  entry: TranscriptHistoryEntry;
  onLoadPreviewText?: (entry: TranscriptHistoryEntry) => Promise<string>;
  onReview?: (origin?: DOMRect) => void;
}) {
  const [open, setOpen] = useState(false);
  const [preview, setPreview] = useState<string>();
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const openDelayRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const closeDelayRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  function clearTimers() {
    if (openDelayRef.current) clearTimeout(openDelayRef.current);
    if (closeDelayRef.current) clearTimeout(closeDelayRef.current);
  }

  async function loadPreview(force = false) {
    if (!onLoadPreviewText || (!force && preview !== undefined) || loading) return;

    setLoading(true);
    setFailed(false);

    try {
      const text = await onLoadPreviewText(entry);
      setPreview(text.trim() || "Empty transcript.");
    } catch {
      setFailed(true);
      setPreview(undefined);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    setPreview(undefined);
    setFailed(false);
    if (onLoadPreviewText) void loadPreview(true);
  }, [entry.outputPath]);

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (next) void loadPreview();
  }

  function scheduleOpen() {
    clearTimers();
    openDelayRef.current = setTimeout(() => handleOpenChange(true), OPEN_DELAY_MS);
  }

  function scheduleClose() {
    clearTimers();
    closeDelayRef.current = setTimeout(() => setOpen(false), CLOSE_DELAY_MS);
  }

  function openImmediately() {
    clearTimers();
    handleOpenChange(true);
  }

  if (!onLoadPreviewText) {
    return (
      <span className="min-w-0 truncate font-medium">{entry.name}</span>
    );
  }

  return (
    <Popover modal={false} open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <Button
          aria-label={`Review recording ${entry.name}`}
          className={cn(
            "h-auto w-full min-w-0 justify-start rounded-sm p-0 text-left font-medium hover:bg-transparent hover:underline",
          )}
          onClick={(event) => {
            event.stopPropagation();
            const row = event.currentTarget.closest("[data-history-entry-row]");
            onReview?.(row?.getBoundingClientRect());
          }}
          onFocus={openImmediately}
          onPointerEnter={scheduleOpen}
          onPointerLeave={scheduleClose}
          size="sm"
          type="button"
          variant="ghost"
        >
          <span className="line-clamp-4 whitespace-normal text-left">
            {preview ?? (loading ? "Loading transcript..." : entry.name)}
          </span>
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-80 p-3"
        onOpenAutoFocus={(event) => event.preventDefault()}
        onPointerEnter={scheduleOpen}
        onPointerLeave={scheduleClose}
        side="right"
      >
        <p className="truncate text-sm font-semibold">{entry.name}</p>
        <p className="mt-0.5 text-xs text-muted-foreground">{formatHistoryDate(entry.createdAt)}</p>
        <div className="mt-3 rounded-md border bg-muted/40 p-2.5">
          {loading ? (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-3 w-full" />
              <Skeleton className="h-3 w-5/6" />
              <Skeleton className="h-3 w-2/3" />
            </div>
          ) : failed ? (
            <p className="text-sm text-muted-foreground">Preview unavailable.</p>
          ) : (
            <p className="line-clamp-8 whitespace-pre-wrap text-sm leading-6 text-foreground/90">
              {preview ?? "Loading preview..."}
            </p>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
