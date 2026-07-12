import { useEffect, useState, type Dispatch, type SetStateAction } from "react";

import type { TranscriptHistoryEntry } from "@/history";
import type { RecordingJobView } from "@/lib/app-types";
import {
  applyRestoredQueuePlaybackPaths,
  clearTerminalQueuePlaybackAdmissions,
  currentPlaybackPaths,
  mergeHistoryPlaybackAdmissions,
  reconcilePlaybackAdmissionLifecycle,
  releaseRecordingPlaybackPaths,
  restoreHistoryPlaybackAdmission,
  restoreQueuePlaybackPaths,
  trimHistoryPlaybackAdmissions,
  type HistoryPlaybackAdmissions,
} from "@/lib/playback-registry";

export function useRegisteredPlayback(
  queue: RecordingJobView[],
  setQueue: Dispatch<SetStateAction<RecordingJobView[]>>,
  history: TranscriptHistoryEntry[],
  selectedHistoryEntry?: TranscriptHistoryEntry,
) {
  const [historyPlaybackAdmissions, setHistoryPlaybackAdmissions] =
    useState<HistoryPlaybackAdmissions>({});

  useEffect(() => {
    const controller = new AbortController();
    void Promise.resolve()
      .then(() => controller.signal.aborted
        ? []
        : restoreQueuePlaybackPaths(queue, { signal: controller.signal }))
      .then((restored) => {
        if (controller.signal.aborted) {
          void releaseRecordingPlaybackPaths(restored.map((entry) => entry.playbackPath));
          return;
        }
        if (!restored.length) return;
        setQueue((current) => applyRestoredQueuePlaybackPaths(current, restored));
      });

    return () => {
      controller.abort();
    };
  }, [queue, setQueue]);

  useEffect(() => {
    const selectedHistory = selectedHistoryEntry
      ? history.filter((entry) => entry.outputPath === selectedHistoryEntry.outputPath)
      : [];
    setHistoryPlaybackAdmissions((current) => (
      trimHistoryPlaybackAdmissions(current, selectedHistory)
    ));
  }, [history, selectedHistoryEntry]);

  useEffect(() => {
    if (
      !selectedHistoryEntry ||
      historyPlaybackAdmissions[selectedHistoryEntry.outputPath]
    ) return;

    const controller = new AbortController();
    void restoreHistoryPlaybackAdmission(selectedHistoryEntry, {
      signal: controller.signal,
    }).then((restored) => {
      if (!restored) return;
      if (controller.signal.aborted) {
        void releaseRecordingPlaybackPaths([restored.playbackPath]);
        return;
      }
      setHistoryPlaybackAdmissions((current) => (
        mergeHistoryPlaybackAdmissions(current, [restored])
      ));
    });

    return () => {
      controller.abort();
    };
  }, [historyPlaybackAdmissions, selectedHistoryEntry]);

  useEffect(() => {
    reconcilePlaybackAdmissionLifecycle(
      currentPlaybackPaths(queue, historyPlaybackAdmissions),
    );
    if (queue.some((item) =>
      (item.status === "cancelled" || item.status === "failed") &&
      (item.playbackPath || item.playbackByteLength !== undefined))) {
      setQueue(clearTerminalQueuePlaybackAdmissions);
    }
  }, [historyPlaybackAdmissions, queue, setQueue]);

  return {
    historyPlaybackAdmissions,
  };
}
