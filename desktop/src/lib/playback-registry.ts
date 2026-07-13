import { invoke, isTauri } from "@tauri-apps/api/core";

import { historyEntryPlaybackPath, type TranscriptHistoryEntry } from "@/history";
import type { PlaybackAdmission, RecordingJobView } from "@/lib/app-types";

export const maxPlaybackRestoreConcurrency = 4;
export const maxWaveformAdmissionBytes = 32 * 1024 * 1024;
export const playbackAdmissionDeadlineMs = 10_000;
const maxWaveformAdmissionBytesExact = BigInt(maxWaveformAdmissionBytes);
const unclaimedAdmissionGraceMs = 5_000;
const runtimePlaybackPathPattern = /^\/media\/[0-9a-f]{64}$/;

export type RestoredHistoryPlaybackAdmission = {
  byteLength: number;
  outputPath: string;
  playbackPath: string;
  sourcePath: string;
};

type HistoryPlaybackAdmission = PlaybackAdmission & {
  sourcePath: string;
};

export type HistoryPlaybackAdmissions = Record<string, HistoryPlaybackAdmission>;

type PlaybackAdmissionTracker = ReturnType<typeof createPlaybackAdmissionTracker>;
type RestorePlayback = (path: string) => Promise<PlaybackAdmission>;
type ReleasePlayback = (playbackPath: string) => Promise<unknown>;

type RestoreOptions = {
  claim?: (playbackPath: string, deadlineAt: number) => void;
  release?: ReleasePlayback;
  restore?: RestorePlayback;
  runtime?: boolean;
  signal?: AbortSignal;
};

type NativeAdmissionTask = {
  admit: () => Promise<PlaybackAdmission>;
  deadlineAt: number;
  releaseLate: ReleasePlayback;
  resolve: (admission: PlaybackAdmission | undefined) => void;
  settled: boolean;
  started: boolean;
  timer?: ReturnType<typeof setTimeout>;
};

const pendingNativeAdmissions: NativeAdmissionTask[] = [];
let activeNativeAdmissions = 0;

function settleNativeAdmission(
  task: NativeAdmissionTask,
  admission: PlaybackAdmission | undefined,
) {
  if (task.settled) return;
  task.settled = true;
  if (task.timer !== undefined) clearTimeout(task.timer);
  task.resolve(admission);
}

function finishNativeAdmission() {
  activeNativeAdmissions -= 1;
  pumpNativeAdmissions();
}

function pumpNativeAdmissions() {
  while (
    activeNativeAdmissions < maxPlaybackRestoreConcurrency
    && pendingNativeAdmissions.length
  ) {
    const task = pendingNativeAdmissions.shift()!;
    if (task.settled || task.deadlineAt <= Date.now()) {
      settleNativeAdmission(task, undefined);
      continue;
    }

    task.started = true;
    activeNativeAdmissions += 1;
    let pending: Promise<PlaybackAdmission>;
    try {
      pending = task.admit();
    } catch {
      finishNativeAdmission();
      settleNativeAdmission(task, undefined);
      continue;
    }

    void Promise.resolve(pending).then(
      async (admission) => {
        const late = task.settled;
        if (late) {
          await Promise.resolve()
            .then(() => task.releaseLate(admission.playbackPath))
            .catch(() => undefined);
        }
        finishNativeAdmission();
        if (!late) settleNativeAdmission(task, admission);
      },
      () => {
        finishNativeAdmission();
        settleNativeAdmission(task, undefined);
      },
    );
  }
}

export function settlePlaybackAdmissionBeforeDeadline(
  admit: () => Promise<PlaybackAdmission>,
  releaseLate: ReleasePlayback,
  deadlineAt = Date.now() + playbackAdmissionDeadlineMs,
) {
  return new Promise<PlaybackAdmission | undefined>((resolve) => {
    if (deadlineAt <= Date.now()) {
      resolve(undefined);
      return;
    }

    const task: NativeAdmissionTask = {
      admit,
      deadlineAt,
      releaseLate,
      resolve,
      settled: false,
      started: false,
    };
    task.timer = setTimeout(() => {
      settleNativeAdmission(task, undefined);
      if (task.started) return;
      const index = pendingNativeAdmissions.indexOf(task);
      if (index >= 0) pendingNativeAdmissions.splice(index, 1);
      pumpNativeAdmissions();
    }, Math.max(0, deadlineAt - Date.now()));
    pendingNativeAdmissions.push(task);
    pumpNativeAdmissions();
  });
}

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
    provisional: boolean;
    revoking: boolean;
    timer?: ReturnType<typeof setTimeout>;
  }>();

  function forget(playbackPath: string) {
    const entry = entries.get(playbackPath);
    if (!entry) return;
    if (entry.timer !== undefined) clearTimeout(entry.timer);
    entries.delete(playbackPath);
  }

  function revokeTracked(playbackPath: string) {
    const entry = entries.get(playbackPath);
    if (!entry || entry.revoking) return;
    if (entry.timer !== undefined) {
      clearTimeout(entry.timer);
      entry.timer = undefined;
    }
    entry.revoking = true;
    let revoked: void | Promise<unknown>;
    try {
      revoked = revoke(playbackPath);
    } catch {
      revoked = Promise.reject(new Error("Playback revocation failed."));
    }
    void Promise.resolve(revoked).then(
      () => {
        if (entries.get(playbackPath) === entry) forget(playbackPath);
      },
      () => {
        if (entries.get(playbackPath) !== entry) return;
        entry.revoking = false;
        if (!entry.claimed) {
          entry.timer = setTimeout(() => revokeTracked(playbackPath), graceMs);
        }
      },
    );
  }

  function claim(playbackPath: string) {
    if (!isRuntimePlaybackPath(playbackPath)) return;
    const entry = entries.get(playbackPath);
    if (entry) {
      entry.claimed = true;
      entry.provisional = false;
      if (entry.timer !== undefined) {
        clearTimeout(entry.timer);
        entry.timer = undefined;
      }
      return;
    }
    entries.set(playbackPath, {
      claimed: true,
      provisional: false,
      revoking: false,
    });
  }

  function hold(playbackPath: string, expiresAt: number) {
    if (!isRuntimePlaybackPath(playbackPath)) return;
    let entry = entries.get(playbackPath);
    if (!entry) {
      entry = { claimed: true, provisional: true, revoking: false };
      entries.set(playbackPath, entry);
    } else if (entry.claimed && !entry.provisional) {
      return;
    } else {
      entry.claimed = true;
      entry.provisional = true;
      if (entry.timer !== undefined) clearTimeout(entry.timer);
    }
    const heldEntry = entry;
    heldEntry.timer = setTimeout(() => {
      if (entries.get(playbackPath) !== heldEntry || !heldEntry.provisional) return;
      heldEntry.timer = undefined;
      heldEntry.claimed = false;
      heldEntry.provisional = false;
      revokeTracked(playbackPath);
    }, Math.max(0, expiresAt - Date.now()));
  }

  return {
    claim,
    dispose() {
      for (const playbackPath of [...entries.keys()]) forget(playbackPath);
    },
    forget,
    reconcile(activePlaybackPaths: Iterable<string>) {
      const active = new Set(
        [...activePlaybackPaths].filter(isRuntimePlaybackPath),
      );
      for (const playbackPath of active) {
        claim(playbackPath);
      }
      for (const [playbackPath, entry] of [...entries]) {
        if (entry.claimed && !entry.provisional && !active.has(playbackPath)) {
          entry.claimed = false;
          revokeTracked(playbackPath);
        }
      }
    },
    hold,
    track(playbackPath: string) {
      if (!isRuntimePlaybackPath(playbackPath) || entries.has(playbackPath)) return;
      const entry: {
        claimed: boolean;
        provisional: boolean;
        revoking: boolean;
        timer?: ReturnType<typeof setTimeout>;
      } = {
        claimed: false,
        provisional: false,
        revoking: false,
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
  await invokeRelease(playbackPath);
  runtimeAdmissionTracker.forget(playbackPath);
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

function restoreDependencies(options: RestoreOptions) {
  const restore = options.restore ?? restoreRecordingPlaybackPath;
  return {
    claim: options.claim ?? (options.restore
      ? () => undefined
      : (playbackPath: string, deadlineAt: number) => runtimeAdmissionTracker.hold(
          playbackPath,
          deadlineAt + unclaimedAdmissionGraceMs,
        )),
    release: options.release ?? releaseRecordingPlaybackPath,
    restore,
    runtime: options.runtime ?? isTauri(),
    signal: options.signal,
  };
}

type RestoreDependencies = ReturnType<typeof restoreDependencies>;
type RestoreConsumer = {
  abortListener?: () => void;
  aborted: boolean;
  resolve: (admission: PlaybackAdmission | undefined) => void;
  signal?: AbortSignal;
};
type RestoreTask = {
  consumers: RestoreConsumer[];
  deadlineAt: number;
  dependencies: RestoreDependencies;
  path: string;
  running: boolean;
};

const pendingRestoreTasks: RestoreTask[] = [];
const restoreTasksByPath = new Map<string, RestoreTask>();
let activeRestoreCount = 0;

function finishRestoreConsumer(
  consumer: RestoreConsumer,
  admission: PlaybackAdmission | undefined,
) {
  if (consumer.abortListener) {
    consumer.signal?.removeEventListener("abort", consumer.abortListener);
  }
  consumer.resolve(consumer.aborted ? undefined : admission);
}

function pumpRestoreTasks() {
  while (
    activeRestoreCount < maxPlaybackRestoreConcurrency &&
    pendingRestoreTasks.length
  ) {
    const task = pendingRestoreTasks.shift()!;
    if (task.consumers.every((consumer) => consumer.aborted)) {
      restoreTasksByPath.delete(task.path);
      task.consumers.forEach((consumer) => finishRestoreConsumer(consumer, undefined));
      continue;
    }

    task.running = true;
    activeRestoreCount += 1;
    void (async () => {
      const admission = await settlePlaybackAdmissionBeforeDeadline(
        () => task.dependencies.restore(task.path),
        task.dependencies.release,
        task.deadlineAt,
      );

      if (restoreTasksByPath.get(task.path) === task) {
        restoreTasksByPath.delete(task.path);
      }
      const abandoned = task.consumers.every((consumer) => consumer.aborted);
      if (admission && abandoned) {
        await task.dependencies.release(admission.playbackPath).catch(() => undefined);
      }
      task.consumers.forEach((consumer) => finishRestoreConsumer(
        consumer,
        abandoned ? undefined : admission,
      ));
      activeRestoreCount -= 1;
      pumpRestoreTasks();
    })();
  }
}

function schedulePlaybackRestore(
  path: string,
  dependencies: RestoreDependencies,
  deadlineAt = Date.now() + playbackAdmissionDeadlineMs,
) {
  if (dependencies.signal?.aborted) return Promise.resolve(undefined);

  return new Promise<PlaybackAdmission | undefined>((resolve) => {
    const consumer: RestoreConsumer = {
      aborted: false,
      resolve,
      signal: dependencies.signal,
    };
    if (dependencies.signal) {
      consumer.abortListener = () => {
        consumer.aborted = true;
        if (!restoreTasksByPath.get(path)?.running) pumpRestoreTasks();
      };
      dependencies.signal.addEventListener("abort", consumer.abortListener, { once: true });
    }

    const existing = restoreTasksByPath.get(path);
    if (existing) {
      existing.consumers.push(consumer);
      return;
    }
    const task: RestoreTask = {
      consumers: [consumer],
      deadlineAt,
      dependencies,
      path,
      running: false,
    };
    restoreTasksByPath.set(path, task);
    pendingRestoreTasks.push(task);
    pumpRestoreTasks();
  });
}

export function trimHistoryPlaybackAdmissions(
  admissions: HistoryPlaybackAdmissions,
  history: TranscriptHistoryEntry[],
) {
  const visibleRecordings = new Map(history.flatMap((entry) => {
    const sourcePath = historyPlaybackSourcePath(entry);
    return sourcePath ? [[entry.outputPath, sourcePath] as const] : [];
  }));
  const next = Object.fromEntries(
    Object.entries(admissions).filter(
      ([outputPath, admission]) => visibleRecordings.get(outputPath) === admission.sourcePath,
    ),
  );
  return Object.keys(next).length === Object.keys(admissions).length ? admissions : next;
}

export function historyPlaybackSourcePath(entry: TranscriptHistoryEntry) {
  return historyEntryPlaybackPath(entry) ??
    (entry.sourcePath !== entry.outputPath ? entry.sourcePath : undefined);
}

export function projectHistoryPlaybackAdmission(
  entry: TranscriptHistoryEntry,
  admissions: HistoryPlaybackAdmissions,
) {
  const admission = admissions[entry.outputPath];
  return admission?.sourcePath === historyPlaybackSourcePath(entry)
    ? admission
    : undefined;
}

export async function restoreHistoryPlaybackAdmission(
  entry: TranscriptHistoryEntry,
  options: RestoreOptions = {},
) {
  const dependencies = restoreDependencies(options);
  if (!dependencies.runtime || dependencies.signal?.aborted) return undefined;
  const path = historyPlaybackSourcePath(entry);
  if (!path) return undefined;

  const admission = await schedulePlaybackRestore(
    path,
    dependencies,
    Date.now() + playbackAdmissionDeadlineMs,
  );
  return admission ? {
    ...admission,
    outputPath: entry.outputPath,
    sourcePath: path,
  } : undefined;
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
      current.byteLength === entry.byteLength &&
      current.sourcePath === entry.sourcePath
    ) continue;
    next[entry.outputPath] = {
      byteLength: entry.byteLength,
      playbackPath: entry.playbackPath,
      sourcePath: entry.sourcePath,
    };
    changed = true;
  }
  return changed ? next : admissions;
}
