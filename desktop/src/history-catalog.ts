import { invoke, isTauri } from "@tauri-apps/api/core";

import type { SavedTranscriptSession } from "@/history";

export type HistoryOrigin = "live" | "remote";

export type NativeHistorySession = SavedTranscriptSession & {
  origin: HistoryOrigin;
};

export type NativeHistoryCatalog = {
  maintenanceWarnings: string[];
  sessions: NativeHistorySession[];
};

export async function loadNativeHistoryCatalog(): Promise<NativeHistoryCatalog> {
  if (!isTauri()) return { maintenanceWarnings: [], sessions: [] };
  return invoke<NativeHistoryCatalog>("history_catalog");
}
