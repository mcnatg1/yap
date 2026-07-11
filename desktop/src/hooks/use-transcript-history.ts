import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import {
  hideTranscriptHistory,
  isNativeLiveTranscriptHistoryEntry,
  pruneMissingHiddenTranscriptHistory,
  readTranscriptHistory,
  readHiddenTranscriptHistory,
  readVisibleTranscriptHistory,
  recordVisibleTranscriptHistoryEntries,
  reconcileNativeTranscriptHistoryEntries,
  removeTranscriptHistory,
  transcriptPathIdentity,
  writeHiddenTranscriptHistory,
  writeTranscriptHistory,
  type HistoryStorage,
  type TranscriptHistoryEntry,
} from "@/history";
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
) => boolean;

export function createTranscriptHistoryStore({
  getCurrentHistory,
  onWarning,
  replaceHistory,
  storage,
}: TranscriptHistoryStoreOptions) {
  let acceptedNativeGeneration = 0;
  const acceptedNativeGenerationByOutput = new Map<
    string,
    { generation: number; session: string }
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
    const next = recordVisibleTranscriptHistoryEntries(
      getCurrentHistory(),
      entries,
      hiddenHistoryOutputs,
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
        acceptedNativeGenerationByOutput.set(
          transcriptPathIdentity(entry.outputPath),
          { generation: acceptedNativeGeneration, session: entry.name },
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
      if (applied) return false;
      applied = true;

      const acceptedAfterBaseline = getCurrentHistory().filter((entry) => {
        if (!isNativeLiveTranscriptHistoryEntry(entry)) return false;
        const metadata = acceptedNativeGenerationByOutput.get(
          transcriptPathIdentity(entry.outputPath),
        );
        return metadata !== undefined
          && metadata.generation > baselineGeneration
          && metadata.session === entry.name;
      });
      const acceptedSessions = new Set(
        acceptedAfterBaseline.map((entry) => entry.name),
      );
      const mergedNativeEntries = [
        ...acceptedAfterBaseline,
        ...entries.filter((entry) => !acceptedSessions.has(entry.name)),
      ];
      const next = reconcileNativeTranscriptHistoryEntries(
        readTranscriptHistory(storage),
        mergedNativeEntries,
        readHiddenTranscriptHistory(storage),
      );
      try {
        writeTranscriptHistory(next, storage);
      } catch (error) {
        onWarning(warning, error);
        return false;
      }

      const visibleHistory = readVisibleTranscriptHistory(storage);
      pruneAcceptedNativeGenerations(visibleHistory);
      replaceHistory(visibleHistory);
      return true;
    };
  };

  return {
    captureNativeHistoryReconciliation,
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
    recordVisibleHistoryEntries,
  } = historyStoreRef.current;

  const reconcileHiddenHistory = useCallback(async () => {
    await pruneMissingHiddenTranscriptHistory(resolveOwnedLiveTranscriptPaths);
    replaceHistory(readVisibleTranscriptHistory());
  }, [replaceHistory]);

  const rememberHiddenHistoryEntry = useCallback((outputPath: string) => {
    const next = hideTranscriptHistory(readHiddenTranscriptHistory(), outputPath);
    try {
      writeHiddenTranscriptHistory(next);
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
      return false;
    }
    replaceHistory(next);
    return true;
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
