import { invoke, isTauri } from "@tauri-apps/api/core";

import { historyEntryPlaybackPath, type TranscriptHistoryEntry } from "@/history";
import type { PlaybackAdmission, RecordingJobView } from "@/lib/app-types";

export type RestoredQueuePlaybackAdmission = {
  byteLength: number;
  id: number;
  playbackPath: string;
};

export type RestoredHistoryPlaybackAdmission = {
  byteLength: number;
  outputPath: string;
  playbackPath: string;
};

export type HistoryPlaybackAdmissions = Record<string, PlaybackAdmission>;

function validatePlaybackAdmission(value: unknown): PlaybackAdmission {
  if (!value || typeof value !== "object") throw new Error("Invalid playback admission.");
  const admission = value as Record<string, unknown>;
  if (
    typeof admission.playbackPath !== "string" ||
    !admission.playbackPath ||
    !Number.isSafeInteger(admission.byteLength) ||
    Number(admission.byteLength) < 0
  ) {
    throw new Error("Invalid playback admission.");
  }
  return {
    byteLength: Number(admission.byteLength),
    playbackPath: admission.playbackPath,
  };
}

export async function allowRecordingPlaybackPath(path: string) {
  if (!isTauri()) return { byteLength: 0, playbackPath: path };
  return validatePlaybackAdmission(await invoke<unknown>("allow_recording_playback_path", { path }));
}

async function restoreRecordingPlaybackPath(path: string) {
  if (!isTauri()) return { byteLength: 0, playbackPath: path };
  return validatePlaybackAdmission(await invoke<unknown>("restore_recording_playback_path", { path }));
}

export async function restoreQueuePlaybackPaths(queue: RecordingJobView[]) {
  if (!isTauri()) return [];
  const missing = queue.filter(
    (item) =>
      item.intent === "recording" &&
      (!item.playbackPath || item.playbackByteLength === undefined),
  );

  const restored = await Promise.all(
    missing.map(async (item) => {
      try {
        const admission = await restoreRecordingPlaybackPath(item.path);
        return { id: item.id, ...admission };
      } catch {
        return undefined;
      }
    }),
  );
  return restored.filter((item): item is RestoredQueuePlaybackAdmission => Boolean(item));
}

export function applyRestoredQueuePlaybackPaths(
  queue: RecordingJobView[],
  restored: RestoredQueuePlaybackAdmission[],
) {
  if (!restored.length) return queue;
  const byId = new Map(restored.map((item) => [item.id, item]));
  return queue.map((item) => {
    const admission = byId.get(item.id);
    return admission
      ? {
          ...item,
          playbackByteLength: admission.byteLength,
          playbackPath: admission.playbackPath,
        }
      : item;
  });
}

export function trimHistoryPlaybackAdmissions(
  admissions: HistoryPlaybackAdmissions,
  history: TranscriptHistoryEntry[],
) {
  const visibleOutputs = new Set(history.map((entry) => entry.outputPath));
  const next = Object.fromEntries(
    Object.entries(admissions).filter(([outputPath]) => visibleOutputs.has(outputPath)),
  );
  return Object.keys(next).length === Object.keys(admissions).length ? admissions : next;
}

export async function restoreHistoryPlaybackAdmissions(
  history: TranscriptHistoryEntry[],
  admissions: HistoryPlaybackAdmissions,
) {
  if (!isTauri()) return [];
  const candidates = history.flatMap((entry) => {
    if (admissions[entry.outputPath]) return [];
    const path = historyEntryPlaybackPath(entry) ??
      (entry.sourcePath !== entry.outputPath ? entry.sourcePath : undefined);
    return path ? [{ entry, path }] : [];
  });

  const restored = await Promise.all(
    candidates.map(async ({ entry, path }) => {
      try {
        const admission = await restoreRecordingPlaybackPath(path);
        return {
          ...admission,
          outputPath: entry.outputPath,
        };
      } catch {
        return undefined;
      }
    }),
  );
  return restored.filter((entry): entry is RestoredHistoryPlaybackAdmission => Boolean(entry));
}

export function mergeHistoryPlaybackAdmissions(
  admissions: HistoryPlaybackAdmissions,
  restored: RestoredHistoryPlaybackAdmission[],
) {
  if (!restored.length) return admissions;

  let changed = false;
  const next = { ...admissions };
  for (const entry of restored) {
    const current = next[entry.outputPath];
    if (
      current?.playbackPath === entry.playbackPath &&
      current.byteLength === entry.byteLength
    ) continue;
    next[entry.outputPath] = {
      byteLength: entry.byteLength,
      playbackPath: entry.playbackPath,
    };
    changed = true;
  }
  return changed ? next : admissions;
}
