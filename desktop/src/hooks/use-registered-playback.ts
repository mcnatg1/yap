import { useEffect, useMemo, useState, type Dispatch, type SetStateAction } from "react";

import type { TranscriptHistoryEntry } from "@/history";
import type { RecordingJobView } from "@/lib/app-types";
import {
  applyRestoredQueuePlaybackPaths,
  mergeHistoryPlaybackAdmissions,
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
    let cancelled = false;
    void restoreQueuePlaybackPaths(queue).then((restored) => {
      if (cancelled || !restored.length) return;
      setQueue((current) => applyRestoredQueuePlaybackPaths(current, restored));
    });

    return () => {
      cancelled = true;
    };
  }, [queue, setQueue]);

  useEffect(() => {
    setHistoryPlaybackAdmissions((current) => trimHistoryPlaybackAdmissions(current, history));
  }, [history]);

  useEffect(() => {
    let cancelled = false;
    void restoreHistoryPlaybackAdmissions(history, historyPlaybackAdmissions).then((restored) => {
      if (cancelled || !restored.length) return;
      setHistoryPlaybackAdmissions((current) => mergeHistoryPlaybackAdmissions(current, restored));
    });

    return () => {
      cancelled = true;
    };
  }, [history, historyPlaybackAdmissions]);

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
