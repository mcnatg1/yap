import { isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef } from "react";
import { toast } from "sonner";

import {
  savedSessionToTranscriptHistoryEntry,
  type TranscriptHistoryEntry,
} from "@/history";
import {
  listRecoverableLiveSessions,
  listSavedLiveSessions,
  listenLiveSessionSaved,
  type RecoverableLiveSession,
  type SavedLiveSession,
  type SavedLiveSessionCatalog,
} from "@/live";

const recoveryWindowMs = 24 * 60 * 60 * 1000;
const historySaveWarning = "Transcript history could not be saved.";
const historySyncWarning = "Live transcript history could not be synced.";

export type LiveHistoryStorePort = {
  captureNativeHistoryReconciliation: () => (
    entries: TranscriptHistoryEntry[],
    warning: string,
  ) => boolean;
  reconcileHiddenHistory: () => Promise<void>;
  recordVisibleHistoryEntries: (entries: TranscriptHistoryEntry[], warning: string) => boolean;
};

export function projectNativeLiveHistory(
  catalog: SavedLiveSessionCatalog,
  recoverable: RecoverableLiveSession[],
) {
  const sessions: SavedLiveSession[] = [
    ...catalog.sessions,
    ...recoverable.map((session) => ({
      createdAtMs: Math.max(0, session.expiresAtMs - recoveryWindowMs),
      name: session.name,
      outputPath: session.audioPartialPath ?? session.journalPartialPath ?? session.name,
      sessionId: session.sessionId,
      sourcePath: session.audioPartialPath ?? session.journalPartialPath ?? session.name,
      warning: session.reason,
      recoveryState: "recoverable" as const,
    })),
  ];

  return {
    entries: sessions.map(savedSessionToTranscriptHistoryEntry),
    maintenanceWarnings: catalog.maintenanceWarnings,
  };
}

export function firstUnshownMaintenanceWarning(
  warnings: string[],
  alreadyShown: boolean,
) {
  return alreadyShown ? undefined : warnings[0];
}

export async function syncNativeLiveHistory({
  captureNativeHistoryReconciliation,
  isCancelled,
  listRecoverableSessions,
  listSavedSessions,
  onMaintenanceWarnings,
}: {
  captureNativeHistoryReconciliation: LiveHistoryStorePort["captureNativeHistoryReconciliation"];
  isCancelled: () => boolean;
  listRecoverableSessions: () => Promise<RecoverableLiveSession[]>;
  listSavedSessions: () => Promise<SavedLiveSessionCatalog>;
  onMaintenanceWarnings: (warnings: string[]) => void;
}) {
  if (isCancelled()) return;
  const applyNativeHistory = captureNativeHistoryReconciliation();
  const [catalog, recoverable] = await Promise.all([
    listSavedSessions(),
    listRecoverableSessions(),
  ]);
  if (isCancelled()) return;

  const { entries, maintenanceWarnings } = projectNativeLiveHistory(catalog, recoverable);
  onMaintenanceWarnings(maintenanceWarnings);
  if (isCancelled()) return;
  applyNativeHistory(entries, historySyncWarning);
}

export function recordSavedLiveSession(
  session: SavedLiveSession,
  recordVisibleHistoryEntries: LiveHistoryStorePort["recordVisibleHistoryEntries"],
  onSaved: (entry: TranscriptHistoryEntry) => void,
) {
  const entry = savedSessionToTranscriptHistoryEntry(session);
  if (!recordVisibleHistoryEntries([entry], historySaveWarning)) return false;
  onSaved(entry);
  return true;
}

export function useLiveHistorySync({
  captureNativeHistoryReconciliation,
  onSaved,
  reconcileHiddenHistory,
  recordVisibleHistoryEntries,
}: LiveHistoryStorePort & {
  onSaved: (entry: TranscriptHistoryEntry) => void;
}) {
  const portsRef = useRef({
    captureNativeHistoryReconciliation,
    onSaved,
    reconcileHiddenHistory,
    recordVisibleHistoryEntries,
  });
  portsRef.current = {
    captureNativeHistoryReconciliation,
    onSaved,
    reconcileHiddenHistory,
    recordVisibleHistoryEntries,
  };
  const maintenanceWarningShownRef = useRef(false);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlistenLiveSaved: (() => void) | undefined;

    void listenLiveSessionSaved((session) => {
      recordSavedLiveSession(
        session,
        (...args) => portsRef.current.recordVisibleHistoryEntries(...args),
        (entry) => portsRef.current.onSaved(entry),
      );
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLiveSaved = stop;
    });

    void portsRef.current.reconcileHiddenHistory()
      .then(() => syncNativeLiveHistory({
        captureNativeHistoryReconciliation: () => (
          portsRef.current.captureNativeHistoryReconciliation()
        ),
        isCancelled: () => cancelled,
        listRecoverableSessions: listRecoverableLiveSessions,
        listSavedSessions: listSavedLiveSessions,
        onMaintenanceWarnings: (warnings) => {
          const maintenanceWarning = firstUnshownMaintenanceWarning(
            warnings,
            maintenanceWarningShownRef.current,
          );
          if (!maintenanceWarning) return;
          maintenanceWarningShownRef.current = true;
          toast.warning(maintenanceWarning);
        },
      }))
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlistenLiveSaved?.();
    };
  }, []);
}
