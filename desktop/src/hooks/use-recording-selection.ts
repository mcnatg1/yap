import { useCallback, useEffect, useMemo, useState } from "react";

import type { TranscriptHistoryEntry } from "@/history-model";
import { useRegisteredPlayback } from "@/hooks/use-registered-playback";
import { isRecordingFinished, type RecordingJobView } from "@/lib/app-types";
import {
  projectHistoryPlaybackAdmission,
  type HistoryPlaybackAdmissions,
} from "@/lib/history-playback";
import { historyEntryToRecordingJob } from "@/lib/history-utils";

type ReviewMorphOrigin = {
  height: number;
  left: number;
  top: number;
  width: number;
};

function withHistoryPlaybackAdmission(
  item: RecordingJobView | undefined,
  entry: TranscriptHistoryEntry | undefined,
  admissions: HistoryPlaybackAdmissions,
) {
  if (!item || !entry || item.outputPath !== entry.outputPath) return item;
  const admission = projectHistoryPlaybackAdmission(entry, admissions);
  return admission ? { ...item, playbackPath: admission.playbackPath } : item;
}

export function useRecordingSelection({
  history,
  queue,
}: {
  history: TranscriptHistoryEntry[];
  queue: RecordingJobView[];
}) {
  const [selectedId, setSelectedId] = useState<string>();
  const [selectedHistoryOutput, setSelectedHistoryOutput] = useState<string>();
  const [reviewMorphOrigin, setReviewMorphOrigin] = useState<ReviewMorphOrigin>();

  const historyJob = useCallback(
    (entry: TranscriptHistoryEntry) => historyEntryToRecordingJob(entry),
    [],
  );
  const selectedHistoryEntry = history.find((entry) => entry.outputPath === selectedHistoryOutput);
  const selectedHistoryItemWithoutPlayback = selectedHistoryEntry
    ? historyJob(selectedHistoryEntry)
    : undefined;
  const { historyPlaybackAdmissions } = useRegisteredPlayback(
    queue,
    history,
    selectedHistoryEntry,
  );
  const selectedHistoryItem = useMemo(
    () => withHistoryPlaybackAdmission(
      selectedHistoryItemWithoutPlayback,
      selectedHistoryEntry,
      historyPlaybackAdmissions,
    ),
    [
      historyPlaybackAdmissions,
      selectedHistoryEntry,
      selectedHistoryItemWithoutPlayback,
    ],
  );
  const selectedItem =
    queue.find((item) => item.id === selectedId) ??
    selectedHistoryItem ??
    [...queue].reverse().find((item) => isRecordingFinished(item.status)) ??
    (history[0] ? historyJob(history[0]) : undefined) ??
    queue[0];

  useEffect(() => {
    if (selectedHistoryOutput) return;

    if (!queue.length) {
      setSelectedId(undefined);
      return;
    }

    if (!selectedId || !queue.some((item) => item.id === selectedId)) {
      setSelectedId(queue[queue.length - 1].id);
    }
  }, [queue, selectedId, selectedHistoryOutput]);

  useEffect(() => {
    if (selectedHistoryOutput && !history.some((entry) => entry.outputPath === selectedHistoryOutput)) {
      setSelectedHistoryOutput(undefined);
    }
  }, [history, selectedHistoryOutput]);

  const closeHistoryReview = useCallback(() => {
    setSelectedHistoryOutput(undefined);
    setReviewMorphOrigin(undefined);
  }, []);

  const clearHistorySelectionIf = useCallback((outputPath: string) => {
    if (selectedHistoryOutput === outputPath) setSelectedHistoryOutput(undefined);
  }, [selectedHistoryOutput]);

  const selectQueueItem = useCallback((id: string) => {
    setSelectedHistoryOutput(undefined);
    setSelectedId(id);
  }, []);

  const selectQueueItemOnly = useCallback((id: string) => {
    setSelectedId(id);
  }, []);

  const selectHistoryEntry = useCallback((entry: TranscriptHistoryEntry, origin?: DOMRect) => {
    setSelectedId(undefined);
    setSelectedHistoryOutput(entry.outputPath);
    setReviewMorphOrigin(
      origin
        ? {
            height: origin.height,
            left: origin.left,
            top: origin.top,
            width: origin.width,
          }
        : undefined,
    );
  }, []);

  return {
    clearHistorySelectionIf,
    closeHistoryReview,
    historyJob,
    reviewMorphOrigin,
    selectHistoryEntry,
    selectQueueItem,
    selectQueueItemOnly,
    selectedHistoryEntry,
    selectedHistoryItem,
    selectedHistoryOutput,
    selectedId,
    selectedItem,
  };
}
