import type { TranscriptHistoryEntry } from "@/history-model";
import type { PlaybackAdmission, RecordingJobView } from "@/lib/recording-job";
import {
  hasNativePlaybackRuntime,
  holdPlaybackAdmissionUntil,
  releaseRecordingPlaybackPath,
  restoreRecordingPlaybackPath,
} from "@/lib/playback-admission";
import {
  maxPlaybackRestoreConcurrency,
  playbackAdmissionDeadlineMs,
  settlePlaybackAdmissionBeforeDeadline,
  type ReleasePlayback,
  type RestorePlayback,
} from "@/lib/playback-admission-queue";
import { historyEntryPlaybackPath } from "@/native-history";

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

type RestoreOptions = {
  claim?: (playbackPath: string, deadlineAt: number) => void;
  release?: ReleasePlayback;
  restore?: RestorePlayback;
  runtime?: boolean;
  signal?: AbortSignal;
};

function restoreDependencies(options: RestoreOptions) {
  const restore = options.restore ?? restoreRecordingPlaybackPath;
  return {
    claim: options.claim ?? (options.restore
      ? () => undefined
      : holdPlaybackAdmissionUntil),
    release: options.release ?? releaseRecordingPlaybackPath,
    restore,
    runtime: options.runtime ?? hasNativePlaybackRuntime(),
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
      task.consumers.forEach((consumer) => finishRestoreConsumer(
        consumer,
        abandoned ? undefined : admission,
      ));
      activeRestoreCount -= 1;
      pumpRestoreTasks();
      if (admission && abandoned) {
        void Promise.resolve()
          .then(() => task.dependencies.release(admission.playbackPath))
          .catch(() => undefined);
      }
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

  const deadlineAt = Date.now() + playbackAdmissionDeadlineMs;
  const admission = await schedulePlaybackRestore(path, dependencies, deadlineAt);
  if (!admission) return undefined;
  dependencies.claim(admission.playbackPath, deadlineAt);
  return {
    ...admission,
    outputPath: entry.outputPath,
    sourcePath: path,
  };
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
