import { useCallback, useRef, useState } from "react";

import type { TranscriptHistoryEntry } from "@/history";

type PreviewTextLoader = (entry: TranscriptHistoryEntry) => Promise<string>;

export function useTranscriptPreview(loadPreviewText: PreviewTextLoader) {
  const [previewEntry, setPreviewEntry] = useState<TranscriptHistoryEntry>();
  const [previewText, setPreviewText] = useState<string | undefined>();
  const previewRequest = useRef(0);

  const previewHistoryEntry = useCallback(async (entry: TranscriptHistoryEntry) => {
    const request = previewRequest.current + 1;
    previewRequest.current = request;
    setPreviewEntry(entry);
    setPreviewText(undefined);

    try {
      const text = await loadPreviewText(entry);
      if (previewRequest.current === request) setPreviewText(text);
    } catch {
      if (previewRequest.current === request) {
        setPreviewText("Preview unavailable. Open the transcript file from the actions menu.");
      }
    }
  }, [loadPreviewText]);

  const closeTranscriptPreview = useCallback(() => {
    previewRequest.current += 1;
    setPreviewEntry(undefined);
    setPreviewText(undefined);
  }, []);

  return {
    closeTranscriptPreview,
    previewEntry,
    previewHistoryEntry,
    previewText,
  };
}
