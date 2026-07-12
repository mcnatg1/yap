import { invoke, isTauri } from "@tauri-apps/api/core";

import { historyEntryPlaybackPath, type TranscriptHistoryEntry } from "@/history";
import type { PlaybackAdmission, RecordingJobView } from "@/lib/app-types";

export const maxPlaybackRestoreConcurrency = 4;
export const maxWaveformAdmissionBytes = 32 * 1024 * 1024;
const maxWaveformAdmissionBytesExact = BigInt(maxWaveformAdmissionBytes);
const unclaimedAdmissionGraceMs = 5_000;
const runtimePlaybackPathPattern = /^\/media\/[0-9a-f]{64}$/;

export type RestoredQueuePlaybackAdmission = {
  byteLength: number;
  id: number;
  playbackPath: string;
  sourcePath?: string;
};

export type RestoredHistoryPlaybackAdmission = {
  byteLength: number;
  outputPath: string;
  playbackPath: string;
  sourcePath?: string;
};

export type HistoryPlaybackAdmissions = Record<string, PlaybackAdmission>;

type PlaybackAdmissionTracker = ReturnType<typeof createPlaybackAdmissionTracker>;
type RestorePlayback = (path: string) => Promise<PlaybackAdmission>;
type ReleasePlayback = (playbackPath: string) => Promise<unknown>;

type RestoreOptions = {
  release?: ReleasePlayback;
  restore?: RestorePlayback;
  runtime?: boolean;
  signal?: AbortSignal;
};

export function validatePlaybackAdmission(value: unknown): PlaybackAdmission {
  if (!value || typeof value !== "object") throw new Error("Invalid playback admission.");
  const admission = value as Record<string, unknown>;
  if (
    typeof admission.playbackPath !== "string" ||
    !isRuntimePlaybackPath(admission.playbackPath) ||
    typeof admission.byteLength !== "string" ||
    !/^(0|[1-9]\d*)$/.test(admission.byteLength) ||
    typeof admission.waveformEligible !== "boolean"
  ) {
    throw new Error("Invalid playback admission.");
  }

  let exactLength: bigint;
  try {
    exactLength = BigInt(admission.byteLength);
  } catch {
    throw new Error("Invalid playback admission.");
  }
  const waveformEligible = admission.waveformEligible &&
    exactLength <= maxWaveformAdmissionBytesExact;

  // RecordingJobView predates native admission metadata. Preserve its safe
  // numeric slot as an eligibility classification, never as a converted u64.
  return {
    byteLength: waveformEligible ? 0 : maxWaveformAdmissionBytes + 1,
    playbackPath: admission.playbackPath,
  };
}

export function createPlaybackAdmissionTracker(
  revoke: (playbackPath: string) => void | Promise<unknown>,
  graceMs = unclaimedAdmissionGraceMs,
) {
  const entries = new Map<string, {
    claimed: boolean;
    timer?: ReturnType<typeof setTimeout>;
  }>();

  function forget(playbackPath: string) {
    const entry = entries.get(playbackPath);
    if (!entry) return;
    if (entry.timer !== undefined) clearTimeout(entry.timer);
    entries.delete(playbackPath);
  }

  function revokeTracked(playbackPath: string) {
    forget(playbackPath);
    void Promise.resolve(revoke(playbackPath)).catch(() => undefined);
  }

  return {
    dispose() {
      for (const playbackPath of [...entries.keys()]) forget(playbackPath);
    },
    forget,
    reconcile(activePlaybackPaths: Iterable<string>) {
      const active = new Set(
        [...activePlaybackPaths].filter(isRuntimePlaybackPath),
      );
      for (const playbackPath of active) {
        const existing = entries.get(playbackPath);
        if (existing) {
          existing.claimed = true;
          if (existing.timer !== undefined) {
            clearTimeout(existing.timer);
            existing.timer = undefined;
          }
        } else {
          entries.set(playbackPath, { claimed: true });
        }
      }
      for (const [playbackPath, entry] of [...entries]) {
        if (entry.claimed && !active.has(playbackPath)) revokeTracked(playbackPath);
      }
    },
    track(playbackPath: string) {
      if (!isRuntimePlaybackPath(playbackPath) || entries.has(playbackPath)) return;
      const entry: { claimed: boolean; timer?: ReturnType<typeof setTimeout> } = {
        claimed: false,
      };
      entry.timer = setTimeout(() => {
        if (entries.get(playbackPath) === entry && !entry.claimed) {
          revokeTracked(playbackPath);
        }
      }, graceMs);
      entries.set(playbackPath, entry);
    },
  };
}

const runtimeAdmissionTracker: PlaybackAdmissionTracker = createPlaybackAdmissionTracker(
  (playbackPath) => invokeRelease(playbackPath),
);

function isRuntimePlaybackPath(playbackPath: string) {
  try {
    const url = new URL(playbackPath);
    return (
      url.protocol === "http:" &&
      url.hostname === "127.0.0.1" &&
      Boolean(url.port) &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash &&
      runtimePlaybackPathPattern.test(url.pathname)
    );
  } catch {
    return false;
  }
}

async function invokeRelease(playbackPath: string) {
  if (!isTauri() || !isRuntimePlaybackPath(playbackPath)) return;
  await invoke("release_recording_playback", { playbackPath });
}

async function admittedPlaybackPath(command: string, path: string) {
  const admission = validatePlaybackAdmission(
    await invoke<unknown>(command, { path }),
  );
  runtimeAdmissionTracker.track(admission.playbackPath);
  return admission;
}

export async function allowRecordingPlaybackPath(path: string) {
  if (!isTauri()) return { byteLength: 0, playbackPath: path };
  return admittedPlaybackPath("allow_recording_playback_path", path);
}

async function restoreRecordingPlaybackPath(path: string) {
  return admittedPlaybackPath("restore_recording_playback_path", path);
}

export async function releaseRecordingPlaybackPath(playbackPath: string) {
  runtimeAdmissionTracker.forget(playbackPath);
  await invokeRelease(playbackPath);
}

export function reconcilePlaybackAdmissionLifecycle(activePlaybackPaths: Iterable<string>) {
  runtimeAdmissionTracker.reconcile(activePlaybackPaths);
}

export function currentPlaybackPaths(
  queue: RecordingJobView[],
  historyAdmissions: HistoryPlaybackAdmissions,
) {
  const paths = new Set<string>();
  for (const item of queue) {
    if (
      item.status !== "cancelled" &&
      item.status !== "failed" &&
      item.playbackPath
    ) {
      paths.add(item.playbackPath);
    }
  }
  for (const admission of Object.values(historyAdmissions)) {
    paths.add(admission.playbackPath);
  }
  return [...paths];
}

export function clearTerminalQueuePlaybackAdmissions(queue: RecordingJobView[]) {
  let changed = false;
  const next = queue.map((item) => {
    if (
      (item.status !== "cancelled" && item.status !== "failed") ||
      (!item.playbackPath && item.playbackByteLength === undefined)
    ) {
      return item;
    }
    const cleared = { ...item };
    delete cleared.playbackPath;
    delete cleared.playbackByteLength;
    changed = true;
    return cleared;
  });
  return changed ? next : queue;
}

export async function releaseRecordingPlaybackPaths(
  playbackPaths: Iterable<string>,
  release: ReleasePlayback = releaseRecordingPlaybackPath,
) {
  const paths = [...new Set(playbackPaths)];
  let cursor = 0;

  async function worker() {
    while (true) {
      const index = cursor;
      cursor += 1;
      if (index >= paths.length) return;
      await release(paths[index]).catch(() => undefined);
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(maxPlaybackRestoreConcurrency, paths.length) },
      () => worker(),
    ),
  );
}

async function restoreBounded<T, R>(
  candidates: T[],
  restore: (candidate: T) => Promise<R>,
  playbackPath: (restored: R) => string,
  options: Required<Pick<RestoreOptions, "release">> & Pick<RestoreOptions, "signal">,
) {
  const results: Array<R | undefined> = new Array(candidates.length);
  let cursor = 0;

  async function worker() {
    while (!options.signal?.aborted) {
      const index = cursor;
      cursor += 1;
      if (index >= candidates.length) return;
      try {
        const restored = await restore(candidates[index]);
        if (options.signal?.aborted) {
          await options.release(playbackPath(restored)).catch(() => undefined);
          continue;
        }
        results[index] = restored;
      } catch {
        // A missing or replaced persisted path is no longer restorable.
      }
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(maxPlaybackRestoreConcurrency, candidates.length) },
      () => worker(),
    ),
  );
  return results.filter((result): result is R => result !== undefined);
}

function restoreDependencies(options: RestoreOptions) {
  return {
    release: options.release ?? releaseRecordingPlaybackPath,
    restore: options.restore ?? restoreRecordingPlaybackPath,
    runtime: options.runtime ?? isTauri(),
    signal: options.signal,
  };
}

export async function restoreQueuePlaybackPaths(
  queue: RecordingJobView[],
  options: RestoreOptions = {},
) {
  const dependencies = restoreDependencies(options);
  if (!dependencies.runtime || dependencies.signal?.aborted) return [];
  const missing = queue.filter(
    (item) =>
      item.intent === "recording" &&
      item.status !== "cancelled" &&
      item.status !== "failed" &&
      (!item.playbackPath || item.playbackByteLength === undefined),
  );

  return restoreBounded(
    missing,
    async (item) => ({
      id: item.id,
      sourcePath: item.path,
      ...await dependencies.restore(item.path),
    }),
    (restored) => restored.playbackPath,
    dependencies,
  );
}

export function applyRestoredQueuePlaybackPaths(
  queue: RecordingJobView[],
  restored: RestoredQueuePlaybackAdmission[],
) {
  if (!restored.length) return queue;
  const byId = new Map(restored.map((item) => [item.id, item]));
  return queue.map((item) => {
    const admission = byId.get(item.id);
    return admission &&
      item.status !== "cancelled" &&
      item.status !== "failed" &&
      (!admission.sourcePath || admission.sourcePath === item.path) &&
      (!item.playbackPath || item.playbackByteLength === undefined)
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
  options: RestoreOptions = {},
) {
  const dependencies = restoreDependencies(options);
  if (!dependencies.runtime || dependencies.signal?.aborted) return [];
  const candidates = history.flatMap((entry) => {
    if (admissions[entry.outputPath]) return [];
    const path = historyEntryPlaybackPath(entry) ??
      (entry.sourcePath !== entry.outputPath ? entry.sourcePath : undefined);
    return path ? [{ entry, path }] : [];
  });

  return restoreBounded(
    candidates,
    async ({ entry, path }) => ({
      ...await dependencies.restore(path),
      outputPath: entry.outputPath,
      sourcePath: path,
    }),
    (restored) => restored.playbackPath,
    dependencies,
  );
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
