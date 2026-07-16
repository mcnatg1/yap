import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import {
  filterHiddenTranscriptHistory,
  filterLegacyHiddenTranscriptHistory,
  hideTranscriptHistory,
  recordVisibleTranscriptHistoryEntries,
  transcriptPathIdentity,
  type TranscriptHistoryEntry,
} from "@/history-model";
import {
  hideNativeHistoryEntry,
  migrateHiddenNativeHistory,
  nativeHistoryIdentity,
} from "@/history-catalog";
import {
  compactHiddenTranscriptHistory,
  pruneMissingHiddenTranscriptHistory,
  readHiddenTranscriptHistory,
  readTranscriptHistory,
  readVisibleTranscriptHistory,
  removeMigratedHiddenTranscriptHistory,
  writeHiddenTranscriptHistory,
  writeTranscriptHistory,
  type HistoryStorage,
} from "@/history-storage";
import {
  isNativeLiveTranscriptHistoryEntry,
  legacyTranscriptHistoryEntries,
  reconcileNativeTranscriptHistoryEntries,
  removeTranscriptHistory,
} from "@/native-history";
import { resolveOwnedLiveTranscriptPaths } from "@/live";

type TranscriptHistoryStoreOptions = {
  getCurrentHistory: () => TranscriptHistoryEntry[];
  onWarning: (warning: string, error: unknown) => void;
  replaceHistory: (next: TranscriptHistoryEntry[]) => void;
  storage?: HistoryStorage;
};

export type NativeHistoryReconciliation = (
  entries: TranscriptHistoryEntry[],
  warning: string,
) => TranscriptHistoryEntry[] | undefined;

export function createTranscriptHistoryStore({
  getCurrentHistory,
  onWarning,
  replaceHistory,
  storage,
}: TranscriptHistoryStoreOptions) {
  let acceptedNativeGeneration = 0;
  let nativeVisibilityAuthorityReady = false;
  const acceptedNativeGenerationByOutput = new Map<
    string,
    { generation: number; sessionId: string }
  >();

  const pruneAcceptedNativeGenerations = (history: TranscriptHistoryEntry[]) => {
    const retainedOutputIdentities = new Set(
      history
        .filter(isNativeLiveTranscriptHistoryEntry)
        .map((entry) => transcriptPathIdentity(entry.outputPath)),
    );
    for (const outputIdentity of acceptedNativeGenerationByOutput.keys()) {
      if (!retainedOutputIdentities.has(outputIdentity)) {
        acceptedNativeGenerationByOutput.delete(outputIdentity);
      }
    }
  };

  const recordVisibleHistoryEntries = (
    entries: TranscriptHistoryEntry[],
    warning: string,
  ) => {
    if (!entries.length) return false;

    const hiddenHistoryOutputs = readHiddenTranscriptHistory(storage);
    const current = nativeVisibilityAuthorityReady
      ? filterLegacyHiddenTranscriptHistory(getCurrentHistory(), hiddenHistoryOutputs)
      : filterHiddenTranscriptHistory(getCurrentHistory(), hiddenHistoryOutputs);
    const visibleEntries = nativeVisibilityAuthorityReady
      ? filterLegacyHiddenTranscriptHistory(entries, hiddenHistoryOutputs)
      : filterHiddenTranscriptHistory(entries, hiddenHistoryOutputs);
    const next = recordVisibleTranscriptHistoryEntries(
      current,
      visibleEntries,
      [],
    );
    if (next === getCurrentHistory()) return false;
    try {
      writeTranscriptHistory(next, storage);
    } catch (error) {
      onWarning(warning, error);
      return false;
    }

    const candidateNativeOutputIdentities = new Set(
      entries
        .filter(isNativeLiveTranscriptHistoryEntry)
        .map((entry) => transcriptPathIdentity(entry.outputPath)),
    );
    const acceptedNativeEntries = next
      .filter(isNativeLiveTranscriptHistoryEntry)
      .filter((entry) => (
        candidateNativeOutputIdentities.has(transcriptPathIdentity(entry.outputPath))
      ));
    if (acceptedNativeEntries.length) {
      acceptedNativeGeneration += 1;
      for (const entry of acceptedNativeEntries) {
        if (!entry.sessionId) continue;
        acceptedNativeGenerationByOutput.set(
          transcriptPathIdentity(entry.outputPath),
          { generation: acceptedNativeGeneration, sessionId: entry.sessionId },
        );
      }
    }
    pruneAcceptedNativeGenerations(next);
    replaceHistory(next);
    return true;
  };

  const captureNativeHistoryReconciliation = (): NativeHistoryReconciliation => {
    const baselineGeneration = acceptedNativeGeneration;
    let applied = false;

    return (entries, warning) => {
      if (applied) return undefined;
      applied = true;

      const acceptedAfterBaseline = getCurrentHistory().filter((entry) => {
        const metadata = acceptedNativeGenerationByOutput.get(
          transcriptPathIdentity(entry.outputPath),
        );
        return metadata !== undefined
          && metadata.generation > baselineGeneration
          && metadata.sessionId === entry.sessionId;
      });
      const acceptedSessions = new Set(
        acceptedAfterBaseline.flatMap((entry) => entry.sessionId ?? []),
      );
      const mergedNativeEntries = [
        ...acceptedAfterBaseline,
        ...entries.filter((entry) => (
          !entry.sessionId || !acceptedSessions.has(entry.sessionId)
        )),
      ];
      const legacyHistory = readTranscriptHistory(storage);
      const hiddenHistory = readHiddenTranscriptHistory(storage);
      try {
        writeTranscriptHistory(
          legacyTranscriptHistoryEntries(legacyHistory, mergedNativeEntries),
          storage,
        );
      } catch (error) {
        onWarning(warning, error);
        return undefined;
      }
      const next = reconcileNativeTranscriptHistoryEntries(
        legacyHistory,
        mergedNativeEntries,
        nativeVisibilityAuthorityReady ? [] : hiddenHistory,
      );

      const visibleHistory = nativeVisibilityAuthorityReady
        ? filterLegacyHiddenTranscriptHistory(next, hiddenHistory)
        : filterHiddenTranscriptHistory(next, hiddenHistory);
      pruneAcceptedNativeGenerations(visibleHistory);
      replaceHistory(visibleHistory);
      return visibleHistory;
    };
  };

  return {
    captureNativeHistoryReconciliation,
    confirmNativeVisibilityAuthority() {
      nativeVisibilityAuthorityReady = true;
    },
    recordVisibleHistoryEntries,
  };
}

export function useTranscriptHistory() {
  const [history, setHistory] = useState<TranscriptHistoryEntry[]>(() => readVisibleTranscriptHistory());
  const historyRef = useRef(history);

  const replaceHistory = useCallback((next: TranscriptHistoryEntry[]) => {
    historyRef.current = next;
    setHistory(next);
  }, []);

  useEffect(() => {
    historyRef.current = history;
  }, [history]);

  const historyStoreRef = useRef<ReturnType<typeof createTranscriptHistoryStore> | null>(null);
  if (!historyStoreRef.current) {
    historyStoreRef.current = createTranscriptHistoryStore({
      getCurrentHistory: () => historyRef.current,
      onWarning: (warning, error) => {
        console.warn(warning, error);
        toast.warning(warning);
      },
      replaceHistory,
    });
  }
  const {
    captureNativeHistoryReconciliation,
    confirmNativeVisibilityAuthority,
    recordVisibleHistoryEntries,
  } = historyStoreRef.current;

  const reconcileHiddenHistory = useCallback(async () => {
    await pruneMissingHiddenTranscriptHistory(resolveOwnedLiveTranscriptPaths);
    const hidden = compactHiddenTranscriptHistory();
    const migration = await migrateHiddenNativeHistory(hidden);
    removeMigratedHiddenTranscriptHistory(migration.migratedOutputPaths);
    confirmNativeVisibilityAuthority();
    replaceHistory(readVisibleTranscriptHistory());
  }, [confirmNativeVisibilityAuthority, replaceHistory]);

  const rememberHiddenHistoryEntry = useCallback(async (entry: TranscriptHistoryEntry) => {
    try {
      if (nativeHistoryIdentity(entry)) {
        await hideNativeHistoryEntry(entry);
      } else {
        const next = hideTranscriptHistory(
          readHiddenTranscriptHistory(),
          entry.outputPath,
        );
        writeHiddenTranscriptHistory(next);
      }
    } catch (error) {
      console.warn("Hidden transcript history could not be saved.", error);
      toast.warning("Hidden transcript history could not be saved.");
      return false;
    }
    return true;
  }, []);

  const forgetHistoryEntry = useCallback((outputPath: string) => {
    const next = removeTranscriptHistory(historyRef.current, outputPath);
    try {
      writeTranscriptHistory(next);
    } catch (error) {
      console.warn("Transcript history removal could not be saved.", error);
      toast.warning("Transcript history removal could not be saved.");
    }
    replaceHistory(next);
  }, [replaceHistory]);

  return {
    captureNativeHistoryReconciliation,
    forgetHistoryEntry,
    history,
    reconcileHiddenHistory,
    recordVisibleHistoryEntries,
    rememberHiddenHistoryEntry,
  };
}
