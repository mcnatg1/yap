import { invoke, isTauri } from "@tauri-apps/api/core";

import { historyEntryPlaybackPath, type TranscriptHistoryEntry } from "@/history";
import type { RecordingJobView } from "@/lib/app-types";

export type RestoredQueuePlaybackPath = {
  id: number;
  playbackPath: string;
};

export type RestoredHistoryPlaybackPath = {
  outputPath: string;
  playbackPath: string;
};

export async function allowRecordingPlaybackPath(path: string) {
  if (!isTauri()) return path;
  return invoke<string>("allow_recording_playback_path", { path });
}

async function restoreRecordingPlaybackPath(path: string) {
  if (!isTauri()) return path;
  return invoke<string>("restore_recording_playback_path", { path });
}

export async function restoreQueuePlaybackPaths(queue: RecordingJobView[]) {
  if (!isTauri()) return [];
  const missing = queue.filter(
    (item) => item.intent === "recording" && !item.playbackPath,
  );

  const restored = await Promise.all(
    missing.map(async (item) => {
      try {
        return { id: item.id, playbackPath: await restoreRecordingPlaybackPath(item.path) };
      } catch {
        return undefined;
      }
    }),
  );
  return restored.filter((item): item is RestoredQueuePlaybackPath => Boolean(item));
}

export function applyRestoredQueuePlaybackPaths(
  queue: RecordingJobView[],
  restored: RestoredQueuePlaybackPath[],
) {
  if (!restored.length) return queue;
  const byId = new Map(restored.map((item) => [item.id, item.playbackPath]));
  return queue.map((item) => {
    const playbackPath = byId.get(item.id);
    return playbackPath ? { ...item, playbackPath } : item;
  });
}

export function trimHistoryPlaybackPaths(
  playbackPaths: Record<string, string>,
  history: TranscriptHistoryEntry[],
) {
  const visibleOutputs = new Set(history.map((entry) => entry.outputPath));
  const next = Object.fromEntries(
    Object.entries(playbackPaths).filter(([outputPath]) => visibleOutputs.has(outputPath)),
  );
  return Object.keys(next).length === Object.keys(playbackPaths).length ? playbackPaths : next;
}

export async function restoreHistoryPlaybackPaths(
  history: TranscriptHistoryEntry[],
  playbackPaths: Record<string, string>,
) {
  if (!isTauri()) return [];
  const candidates = history.filter(
    (entry) =>
      !playbackPaths[entry.outputPath] &&
      !historyEntryPlaybackPath(entry) &&
      entry.sourcePath !== entry.outputPath,
  );

  const restored = await Promise.all(
    candidates.map(async (entry) => {
      try {
        return {
          outputPath: entry.outputPath,
          playbackPath: await restoreRecordingPlaybackPath(entry.sourcePath),
        };
      } catch {
        return undefined;
      }
    }),
  );
  return restored.filter((entry): entry is RestoredHistoryPlaybackPath => Boolean(entry));
}

export function mergeHistoryPlaybackPaths(
  playbackPaths: Record<string, string>,
  restored: RestoredHistoryPlaybackPath[],
) {
  if (!restored.length) return playbackPaths;

  let changed = false;
  const next = { ...playbackPaths };
  for (const entry of restored) {
    if (next[entry.outputPath] === entry.playbackPath) continue;
    next[entry.outputPath] = entry.playbackPath;
    changed = true;
  }
  return changed ? next : playbackPaths;
}
