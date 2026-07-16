import { invoke, isTauri } from "@tauri-apps/api/core";

import type { TranscriptHistoryEntry } from "@/history-model";
import type { SavedTranscriptSession } from "@/native-history";

export type HistoryOrigin = "live" | "remote";

export type NativeHistorySession = SavedTranscriptSession & {
  origin: HistoryOrigin;
};

export type NativeHistoryCatalog = {
  maintenanceWarnings: string[];
  sessions: NativeHistorySession[];
};

export type NativeHistoryIdentity = {
  origin: HistoryOrigin;
  outputPath: string;
  sessionId: string;
};

type HiddenHistoryMigration = {
  migratedOutputPaths: string[];
};

export function nativeHistoryIdentity(
  entry: TranscriptHistoryEntry,
): NativeHistoryIdentity | undefined {
  if (!entry.origin || !entry.sessionId || !/^[a-z0-9_-]{1,128}$/i.test(entry.sessionId)) {
    return undefined;
  }
  return {
    origin: entry.origin,
    outputPath: entry.outputPath,
    sessionId: entry.sessionId,
  };
}

export async function loadNativeHistoryCatalog(): Promise<NativeHistoryCatalog> {
  if (!isTauri()) return { maintenanceWarnings: [], sessions: [] };
  return invoke<NativeHistoryCatalog>("history_catalog");
}

export async function hideNativeHistoryEntry(entry: TranscriptHistoryEntry) {
  const identity = nativeHistoryIdentity(entry);
  if (!identity) throw new Error("Native history identity is unavailable.");
  if (!isTauri()) throw new Error("Native history is unavailable outside the desktop app.");
  await invoke("history_hide_native", { identity });
}

export async function migrateHiddenNativeHistory(
  outputPaths: string[],
): Promise<HiddenHistoryMigration> {
  if (!isTauri()) return { migratedOutputPaths: [] };
  return invoke<HiddenHistoryMigration>("history_migrate_hidden_paths", { outputPaths });
}
