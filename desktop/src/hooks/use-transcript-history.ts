import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import {
  hideTranscriptHistory,
  pruneMissingHiddenTranscriptHistory,
  readTranscriptHistory,
  readHiddenTranscriptHistory,
  readVisibleTranscriptHistory,
  recordVisibleTranscriptHistoryEntries,
  reconcileNativeTranscriptHistoryEntries,
  removeTranscriptHistory,
  writeHiddenTranscriptHistory,
  writeTranscriptHistory,
  type TranscriptHistoryEntry,
} from "@/history";
import { resolveOwnedLiveTranscriptPaths } from "@/live";

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

  const reconcileHiddenHistory = useCallback(async () => {
    await pruneMissingHiddenTranscriptHistory(resolveOwnedLiveTranscriptPaths);
    replaceHistory(readVisibleTranscriptHistory());
  }, [replaceHistory]);

  const recordVisibleHistoryEntries = useCallback((entries: TranscriptHistoryEntry[], warning: string) => {
    if (!entries.length) return false;

    const hiddenHistoryOutputs = readHiddenTranscriptHistory();
    const next = recordVisibleTranscriptHistoryEntries(historyRef.current, entries, hiddenHistoryOutputs);
    if (next === historyRef.current) return false;
    try {
      writeTranscriptHistory(next);
    } catch (error) {
      console.warn(warning, error);
      toast.warning(warning);
      return false;
    }
    replaceHistory(next);
    return true;
  }, [replaceHistory]);

  const reconcileNativeHistoryEntries = useCallback((entries: TranscriptHistoryEntry[], warning: string) => {
    const hiddenHistoryOutputs = readHiddenTranscriptHistory();
    const next = reconcileNativeTranscriptHistoryEntries(
      readTranscriptHistory(),
      entries,
      hiddenHistoryOutputs,
    );
    try {
      writeTranscriptHistory(next);
    } catch (error) {
      console.warn(warning, error);
      toast.warning(warning);
      return false;
    }
    replaceHistory(readVisibleTranscriptHistory());
    return true;
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
    forgetHistoryEntry,
    history,
    reconcileNativeHistoryEntries,
    reconcileHiddenHistory,
    recordVisibleHistoryEntries,
    rememberHiddenHistoryEntry,
  };
}
