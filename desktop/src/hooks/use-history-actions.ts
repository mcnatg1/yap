import { useCallback, useRef } from "react";
import { toast } from "sonner";

import {
  savedSessionToTranscriptHistoryEntry,
  type TranscriptHistoryEntry,
} from "@/history";
import {
  deleteRecoverableLiveSession,
  deleteSavedLiveSession,
  recoverLiveSession,
  type SavedLiveSession,
} from "@/live";

const historySaveWarning = "Transcript history could not be saved.";

export type HistoryActionPorts = {
  clearHistorySelectionIf: (outputPath: string) => void;
  forgetHistoryEntry: (outputPath: string) => boolean;
  forgetTranscriptText: (outputPath: string) => void;
  recordVisibleHistoryEntries: (entries: TranscriptHistoryEntry[], warning: string) => boolean;
  rememberHiddenHistoryEntry: (outputPath: string) => boolean;
  selectHistoryEntry: (entry: TranscriptHistoryEntry) => void;
};

export type HistoryActionRuntime = {
  deleteRecoverableLiveSession: (sessionId: string) => Promise<void>;
  deleteSavedLiveSession: (sessionId: string) => Promise<void>;
  recoverLiveSession: (sessionId: string) => Promise<SavedLiveSession>;
  showError: (message: string) => void;
  showSuccess: (message: string) => void;
};

const nativeHistoryActionRuntime: HistoryActionRuntime = {
  deleteRecoverableLiveSession,
  deleteSavedLiveSession,
  recoverLiveSession,
  showError: (message) => toast.error(message),
  showSuccess: (message) => toast.success(message),
};

function historySessionId(entry: TranscriptHistoryEntry) {
  return entry.name.replace(/^live-/, "");
}

export function runHideHistoryEntry(
  outputPath: string,
  ports: HistoryActionPorts,
  runtime: HistoryActionRuntime = nativeHistoryActionRuntime,
) {
  if (!ports.rememberHiddenHistoryEntry(outputPath)) return;
  if (!ports.forgetHistoryEntry(outputPath)) return;
  ports.clearHistorySelectionIf(outputPath);
  runtime.showSuccess("Hidden from history");
}

export async function runDeleteSavedHistoryEntry(
  entry: TranscriptHistoryEntry,
  ports: HistoryActionPorts,
  runtime: HistoryActionRuntime = nativeHistoryActionRuntime,
) {
  try {
    await runtime.deleteSavedLiveSession(historySessionId(entry));
    if (!ports.rememberHiddenHistoryEntry(entry.outputPath)) return;
    if (!ports.forgetHistoryEntry(entry.outputPath)) return;
    ports.clearHistorySelectionIf(entry.outputPath);
    ports.forgetTranscriptText(entry.outputPath);
    runtime.showSuccess("Deleted from device");
  } catch (error) {
    runtime.showError(String(error || "Delete failed"));
  }
}

export async function runRecoverHistoryEntry(
  entry: TranscriptHistoryEntry,
  ports: HistoryActionPorts,
  runtime: HistoryActionRuntime = nativeHistoryActionRuntime,
) {
  try {
    const saved = await runtime.recoverLiveSession(historySessionId(entry));
    const recovered = savedSessionToTranscriptHistoryEntry(saved);
    if (!ports.recordVisibleHistoryEntries([recovered], historySaveWarning)) return;
    ports.forgetHistoryEntry(entry.outputPath);
    ports.clearHistorySelectionIf(entry.outputPath);
    ports.selectHistoryEntry(recovered);
    runtime.showSuccess("Partial recording recovered");
  } catch (error) {
    runtime.showError(String(error || "Recovery failed"));
  }
}

export async function runDeleteRecoverableHistoryEntry(
  entry: TranscriptHistoryEntry,
  ports: HistoryActionPorts,
  runtime: HistoryActionRuntime = nativeHistoryActionRuntime,
) {
  try {
    await runtime.deleteRecoverableLiveSession(historySessionId(entry));
    if (!ports.forgetHistoryEntry(entry.outputPath)) return;
    ports.clearHistorySelectionIf(entry.outputPath);
    ports.forgetTranscriptText(entry.outputPath);
    runtime.showSuccess("Partial recording deleted");
  } catch (error) {
    runtime.showError(String(error || "Delete failed"));
  }
}

export function useHistoryActions(ports: HistoryActionPorts) {
  const portsRef = useRef(ports);
  portsRef.current = ports;

  const hideHistoryEntry = useCallback((outputPath: string) => {
    runHideHistoryEntry(outputPath, portsRef.current);
  }, []);

  const deleteHistoryEntry = useCallback(async (entry: TranscriptHistoryEntry) => {
    await runDeleteSavedHistoryEntry(entry, portsRef.current);
  }, []);

  const recoverHistoryEntry = useCallback(async (entry: TranscriptHistoryEntry) => {
    await runRecoverHistoryEntry(entry, portsRef.current);
  }, []);

  const deleteRecoverableHistoryEntry = useCallback(async (entry: TranscriptHistoryEntry) => {
    await runDeleteRecoverableHistoryEntry(entry, portsRef.current);
  }, []);

  return {
    deleteHistoryEntry,
    deleteRecoverableHistoryEntry,
    hideHistoryEntry,
    recoverHistoryEntry,
  };
}
