import { useEffect, useState } from "react";

import type { TranscriptHistoryEntry } from "@/history-model";
import type { RecordingJobView } from "@/lib/app-types";
import {
  currentPlaybackPaths,
  mergeHistoryPlaybackAdmissions,
  restoreHistoryPlaybackAdmission,
  trimHistoryPlaybackAdmissions,
  type HistoryPlaybackAdmissions,
} from "@/lib/history-playback";
import {
  reconcilePlaybackAdmissionLifecycle,
  releaseRecordingPlaybackPaths,
} from "@/lib/playback-admission";

export function useRegisteredPlayback(
  queue: RecordingJobView[],
  history: TranscriptHistoryEntry[],
  selectedHistoryEntry?: TranscriptHistoryEntry,
) {
  const [historyPlaybackAdmissions, setHistoryPlaybackAdmissions] =
    useState<HistoryPlaybackAdmissions>({});

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
  }, [historyPlaybackAdmissions, queue]);

  return {
    historyPlaybackAdmissions,
  };
}
