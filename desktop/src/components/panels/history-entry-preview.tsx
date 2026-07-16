import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import type { TranscriptHistoryEntry } from "@/history-model";

export function HistoryEntryPreview({
  entry,
  onLoadPreviewText,
  onReview,
}: {
  entry: TranscriptHistoryEntry;
  onLoadPreviewText?: (entry: TranscriptHistoryEntry) => Promise<string>;
  onReview?: (origin?: DOMRect) => void;
}) {
  const [preview, setPreview] = useState<string>();
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [visible, setVisible] = useState(false);
  const buttonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    setVisible(false);
  }, [entry.outputPath]);

  useEffect(() => {
    if (!onLoadPreviewText || visible) return;
    const button = buttonRef.current;
    if (!button || !("IntersectionObserver" in window)) {
      setVisible(true);
      return;
    }

    const observer = new IntersectionObserver(
      ([item]) => {
        if (!item?.isIntersecting) return;
        setVisible(true);
        observer.disconnect();
      },
      { rootMargin: "240px 0px" },
    );
    observer.observe(button);
    return () => observer.disconnect();
  }, [entry.outputPath, onLoadPreviewText, visible]);

  useEffect(() => {
    let cancelled = false;

    setPreview(undefined);
    setFailed(false);
    if (!onLoadPreviewText || !visible) return;

    setLoading(true);
    void onLoadPreviewText(entry)
      .then((text) => {
        if (cancelled) return;
        setPreview(text.trim() || "Empty transcript.");
      })
      .catch(() => {
        if (cancelled) return;
        setFailed(true);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [entry, onLoadPreviewText, visible]);

  if (!onLoadPreviewText) {
    return (
      <span className="min-w-0 truncate font-medium">{entry.name}</span>
    );
  }

  const label = failed
    ? entry.name
    : preview ?? (loading ? "Loading transcript..." : entry.name);

  return (
    <Button
      ref={buttonRef}
      aria-label={`Review recording ${entry.name}`}
      className="h-auto w-full min-w-0 justify-start rounded-sm p-0 text-left font-medium hover:bg-transparent hover:underline"
      onClick={(event) => {
        event.stopPropagation();
        const row = event.currentTarget.closest("[data-history-entry-row]");
        onReview?.(row?.getBoundingClientRect());
      }}
      size="sm"
      type="button"
      variant="ghost"
    >
      <span className="line-clamp-4 whitespace-normal text-left">{label}</span>
    </Button>
  );
}
