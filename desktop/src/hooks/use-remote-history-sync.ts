import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { toast } from "sonner";

import { savedSessionToTranscriptHistoryEntry, type TranscriptHistoryEntry } from "@/history";
import { createRecordingJobsRefreshCoordinator } from "@/recording-jobs-refresh";
import {
  completedRemoteTranscripts,
  type CompletedRemoteTranscriptCatalog,
} from "@/recording-queue";

const remoteHistorySaveWarning = "Private-server transcript history could not be saved.";

export function projectCompletedRemoteHistory(catalog: CompletedRemoteTranscriptCatalog) {
  return catalog.sessions.map(savedSessionToTranscriptHistoryEntry);
}

export function remoteHistoryResultKey(entry: TranscriptHistoryEntry) {
  return `${entry.sessionId}\0${entry.outputPath}`;
}

export function recordCompletedRemoteHistory(
  catalog: CompletedRemoteTranscriptCatalog,
  recordVisibleHistoryEntries: (entries: TranscriptHistoryEntry[], warning: string) => boolean,
  onSaved?: (entry: TranscriptHistoryEntry) => void,
  recordedResultKeys: ReadonlySet<string> = new Set(),
) {
  const entries = projectCompletedRemoteHistory(catalog);
  if (!entries.length) return true;
  if (!recordVisibleHistoryEntries(entries, remoteHistorySaveWarning)) return false;
  const newlySaved = entries.find(
    (entry) => !recordedResultKeys.has(remoteHistoryResultKey(entry)),
  );
  if (onSaved && newlySaved) onSaved(newlySaved);
  return true;
}

export function useRemoteHistorySync({
  onSaved,
  recordVisibleHistoryEntries,
}: {
  onSaved: (entry: TranscriptHistoryEntry) => void;
  recordVisibleHistoryEntries: (
    entries: TranscriptHistoryEntry[],
    warning: string,
  ) => boolean;
}) {
  const portsRef = useRef({ onSaved, recordVisibleHistoryEntries });
  portsRef.current = { onSaved, recordVisibleHistoryEntries };

  useEffect(() => {
    if (!isTauri()) return;

    let active = true;
    let initialized = false;
    let maintenanceWarningShown = false;
    const recordedResultKeys = new Set<string>();
    let unlisten: (() => void) | undefined;
    const refresh = createRecordingJobsRefreshCoordinator(
      completedRemoteTranscripts,
      (catalog) => {
        if (!active) return;
        if (!maintenanceWarningShown && catalog.maintenanceWarnings[0]) {
          maintenanceWarningShown = true;
          toast.warning(catalog.maintenanceWarnings[0]);
        }
        const notify = initialized ? portsRef.current.onSaved : undefined;
        const recorded = recordCompletedRemoteHistory(
          catalog,
          (...args) => portsRef.current.recordVisibleHistoryEntries(...args),
          notify,
          recordedResultKeys,
        );
        if (recorded) {
          for (const entry of projectCompletedRemoteHistory(catalog)) {
            recordedResultKeys.add(remoteHistoryResultKey(entry));
          }
          initialized = true;
        }
      },
    ).refresh;

    void listen("recording-jobs-changed", () => {
      if (!active) return;
      void refresh().catch(() => {
        if (active) toast.error("Private-server transcript history could not be synced.");
      });
    }).then((stop) => {
      if (!active) {
        stop();
        return;
      }
      unlisten = stop;
      void refresh().catch(() => {
        if (active) toast.error("Private-server transcript history could not be synced.");
      });
    });

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);
}
