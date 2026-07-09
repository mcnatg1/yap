import { useEffect, useState, type Dispatch, type SetStateAction } from "react";

import type { TranscriptHistoryEntry } from "@/history";
import type { RecordingJobView } from "@/lib/app-types";
import {
  applyRestoredQueuePlaybackPaths,
  mergeHistoryPlaybackPaths,
  restoreHistoryPlaybackPaths,
  restoreQueuePlaybackPaths,
  trimHistoryPlaybackPaths,
} from "@/lib/playback-registry";

export function useRegisteredPlayback(
  queue: RecordingJobView[],
  setQueue: Dispatch<SetStateAction<RecordingJobView[]>>,
  history: TranscriptHistoryEntry[],
) {
  const [historyPlaybackPaths, setHistoryPlaybackPaths] = useState<Record<string, string>>({});

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
    setHistoryPlaybackPaths((current) => trimHistoryPlaybackPaths(current, history));
  }, [history]);

  useEffect(() => {
    let cancelled = false;
    void restoreHistoryPlaybackPaths(history, historyPlaybackPaths).then((restored) => {
      if (cancelled || !restored.length) return;
      setHistoryPlaybackPaths((current) => mergeHistoryPlaybackPaths(current, restored));
    });

    return () => {
      cancelled = true;
    };
  }, [history, historyPlaybackPaths]);

  return historyPlaybackPaths;
}
