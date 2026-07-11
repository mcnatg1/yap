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
  reconcileHiddenHistory: () => Promise<void>;
  reconcileNativeHistoryEntries: (entries: TranscriptHistoryEntry[], warning: string) => boolean;
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

export async function loadStableNativeLiveHistory({
  getSavedGeneration,
  isCancelled,
  listRecoverableSessions,
  listSavedSessions,
}: {
  getSavedGeneration: () => number;
  isCancelled: () => boolean;
  listRecoverableSessions: () => Promise<RecoverableLiveSession[]>;
  listSavedSessions: () => Promise<SavedLiveSessionCatalog>;
}) {
  while (true) {
    const savedGeneration = getSavedGeneration();
    const [catalog, recoverable] = await Promise.all([
      listSavedSessions(),
      listRecoverableSessions(),
    ]);
    if (isCancelled()) return undefined;
    if (savedGeneration === getSavedGeneration()) {
      return projectNativeLiveHistory(catalog, recoverable);
    }
  }
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
  onSaved,
  reconcileHiddenHistory,
  reconcileNativeHistoryEntries,
  recordVisibleHistoryEntries,
}: LiveHistoryStorePort & {
  onSaved: (entry: TranscriptHistoryEntry) => void;
}) {
  const portsRef = useRef({
    onSaved,
    reconcileHiddenHistory,
    reconcileNativeHistoryEntries,
    recordVisibleHistoryEntries,
  });
  portsRef.current = {
    onSaved,
    reconcileHiddenHistory,
    reconcileNativeHistoryEntries,
    recordVisibleHistoryEntries,
  };
  const maintenanceWarningShownRef = useRef(false);
  const savedGenerationRef = useRef(0);

  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlistenLiveSaved: (() => void) | undefined;

    void listenLiveSessionSaved((session) => {
      recordSavedLiveSession(
        session,
        (...args) => portsRef.current.recordVisibleHistoryEntries(...args),
        (entry) => {
          savedGenerationRef.current += 1;
          portsRef.current.onSaved(entry);
        },
      );
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLiveSaved = stop;
    });

    void portsRef.current.reconcileHiddenHistory()
      .then(() => loadStableNativeLiveHistory({
        getSavedGeneration: () => savedGenerationRef.current,
        isCancelled: () => cancelled,
        listRecoverableSessions: listRecoverableLiveSessions,
        listSavedSessions: listSavedLiveSessions,
      }))
      .then((projection) => {
        if (cancelled || !projection) return;
        const { entries, maintenanceWarnings } = projection;
        const maintenanceWarning = firstUnshownMaintenanceWarning(
          maintenanceWarnings,
          maintenanceWarningShownRef.current,
        );
        if (maintenanceWarning) {
          maintenanceWarningShownRef.current = true;
          toast.warning(maintenanceWarning);
        }
        portsRef.current.reconcileNativeHistoryEntries(entries, historySyncWarning);
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlistenLiveSaved?.();
    };
  }, []);
}
