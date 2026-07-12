import { useEffect, useMemo, useState, type Dispatch, type SetStateAction } from "react";

import type { TranscriptHistoryEntry } from "@/history";
import type { RecordingJobView } from "@/lib/app-types";
import {
  applyRestoredQueuePlaybackPaths,
  clearTerminalQueuePlaybackAdmissions,
  currentPlaybackPaths,
  mergeHistoryPlaybackAdmissions,
  reconcilePlaybackAdmissionLifecycle,
  releaseRecordingPlaybackPaths,
  restoreHistoryPlaybackAdmissions,
  restoreQueuePlaybackPaths,
  trimHistoryPlaybackAdmissions,
  type HistoryPlaybackAdmissions,
} from "@/lib/playback-registry";

export function useRegisteredPlayback(
  queue: RecordingJobView[],
  setQueue: Dispatch<SetStateAction<RecordingJobView[]>>,
  history: TranscriptHistoryEntry[],
) {
  const [historyPlaybackAdmissions, setHistoryPlaybackAdmissions] =
    useState<HistoryPlaybackAdmissions>({});

  useEffect(() => {
    const controller = new AbortController();
    void restoreQueuePlaybackPaths(queue, { signal: controller.signal }).then((restored) => {
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
    setHistoryPlaybackAdmissions((current) => trimHistoryPlaybackAdmissions(current, history));
  }, [history]);

  useEffect(() => {
    const controller = new AbortController();
    void restoreHistoryPlaybackAdmissions(
      history,
      historyPlaybackAdmissions,
      { signal: controller.signal },
    ).then((restored) => {
      if (controller.signal.aborted) {
        void releaseRecordingPlaybackPaths(restored.map((entry) => entry.playbackPath));
        return;
      }
      if (!restored.length) return;
      setHistoryPlaybackAdmissions((current) => mergeHistoryPlaybackAdmissions(current, restored));
    });

    return () => {
      controller.abort();
    };
  }, [history, historyPlaybackAdmissions]);

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

  return useMemo(() => ({
    historyPlaybackByteLengths: Object.fromEntries(
      Object.entries(historyPlaybackAdmissions).map(([outputPath, admission]) => [
        outputPath,
        admission.byteLength,
      ]),
    ),
    historyPlaybackPaths: Object.fromEntries(
      Object.entries(historyPlaybackAdmissions).map(([outputPath, admission]) => [
        outputPath,
        admission.playbackPath,
      ]),
    ),
  }), [historyPlaybackAdmissions]);
}
