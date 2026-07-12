import { maxTranscriptHistoryEntries, type TranscriptHistoryEntry } from "@/history";
import { boundTextForCache, rememberText } from "@/lib/text-cache";

export type PreviewTextEntry = {
  outputPath: string;
};

export type PreviewTextCache = Record<string, string>;
export const minTranscriptBodySearchLength = 2;
export const historyPreviewSearchBatchSize = 20;
export const historyPreviewSearchConcurrency = 8;
export const historyPreviewSearchFlushDelayMs = 25;

export type PreviewSearchBatch = {
  failedOutputPaths: string[];
  loaded: Array<{ outputPath: string; text: string }>;
};

export type PreviewSearchFailureState = {
  paths: ReadonlySet<string>;
};

export function mergePreviewSearchFailures({
  current,
  failedOutputPaths,
  loadedOutputPaths,
  visibleOutputPaths,
}: {
  current: PreviewSearchFailureState;
  failedOutputPaths: readonly string[];
  loadedOutputPaths: readonly string[];
  visibleOutputPaths: ReadonlySet<string>;
}) {
  const loaded = new Set(loadedOutputPaths);
  const retainedCurrentPaths = [...current.paths].filter(
    (path) => visibleOutputPaths.has(path) && !loaded.has(path),
  );
  const failed = failedOutputPaths.filter(
    (path) => visibleOutputPaths.has(path) && !loaded.has(path),
  );
  const nextPaths = new Set(retainedCurrentPaths);
  failed.forEach((path) => nextPaths.add(path));

  if (
    nextPaths.size === current.paths.size
    && [...nextPaths].every((path) => current.paths.has(path))
  ) {
    return current;
  }
  return { paths: nextPaths };
}

export function prunePreviewSearchFailures(
  current: PreviewSearchFailureState,
  visibleOutputPaths: ReadonlySet<string>,
) {
  const paths = new Set([...current.paths].filter((path) => visibleOutputPaths.has(path)));
  return paths.size === current.paths.size ? current : { paths };
}

type PreviewSearchLoadOptions<Entry extends PreviewTextEntry> = {
  entries: readonly Entry[];
  loadText: (entry: Entry) => Promise<string>;
  onBatch: (batch: PreviewSearchBatch) => void;
  signal?: AbortSignal;
};

type LoadOutcome =
  | { status: "cancelled" }
  | { status: "failed" }
  | { status: "loaded"; text: string };

type ScheduledRead = {
  start: () => void;
};

function createPreviewReadScheduler(concurrency: number) {
  const queue: ScheduledRead[] = [];
  let activeReads = 0;

  function drain() {
    while (activeReads < concurrency && queue.length > 0) {
      queue.shift()?.start();
    }
  }

  function read(loadText: () => Promise<string>, signal?: AbortSignal) {
    return new Promise<LoadOutcome>((resolve) => {
      if (signal?.aborted) {
        resolve({ status: "cancelled" });
        return;
      }

      let settled = false;
      let started = false;
      let scheduledRead: ScheduledRead;

      const settle = (outcome: LoadOutcome) => {
        if (settled) return;
        settled = true;
        signal?.removeEventListener("abort", onAbort);
        resolve(outcome);
      };
      const onAbort = () => {
        settle({ status: "cancelled" });
        if (started) return;
        const index = queue.indexOf(scheduledRead);
        if (index >= 0) queue.splice(index, 1);
        drain();
      };

      scheduledRead = {
        start() {
          if (signal?.aborted) {
            settle({ status: "cancelled" });
            return;
          }

          started = true;
          activeReads += 1;
          let pending: Promise<string>;
          try {
            pending = Promise.resolve(loadText());
          } catch (error) {
            pending = Promise.reject(error);
          }
          void pending
            .then(
              (text) => settle({ status: "loaded", text }),
              () => settle({ status: "failed" }),
            )
            .then(() => {
              activeReads -= 1;
              drain();
            });
        },
      };

      signal?.addEventListener("abort", onAbort, { once: true });
      queue.push(scheduledRead);
      drain();
    });
  }

  return { read };
}

async function loadPreviewSearchEntriesWithScheduler<Entry extends PreviewTextEntry>(
  {
    entries,
    loadText,
    onBatch,
    signal,
  }: PreviewSearchLoadOptions<Entry>,
  scheduler: ReturnType<typeof createPreviewReadScheduler>,
) {
  let batch: PreviewSearchBatch = { failedOutputPaths: [], loaded: [] };
  let flushTimeout: ReturnType<typeof setTimeout> | undefined;
  let nextIndex = 0;

  function clearFlushTimeout() {
    if (flushTimeout === undefined) return;
    clearTimeout(flushTimeout);
    flushTimeout = undefined;
  }

  function publishBatch() {
    clearFlushTimeout();
    if (
      signal?.aborted
      || (batch.loaded.length === 0 && batch.failedOutputPaths.length === 0)
    ) {
      batch = { failedOutputPaths: [], loaded: [] };
      return;
    }
    const completed = batch;
    batch = { failedOutputPaths: [], loaded: [] };
    onBatch(completed);
  }

  function queueBatchFlush() {
    if (flushTimeout !== undefined) return;
    flushTimeout = setTimeout(publishBatch, historyPreviewSearchFlushDelayMs);
  }

  const discardBatch = () => {
    clearFlushTimeout();
    batch = { failedOutputPaths: [], loaded: [] };
  };
  signal?.addEventListener("abort", discardBatch, { once: true });

  async function loadNext() {
    while (!signal?.aborted && nextIndex < entries.length) {
      const entry = entries[nextIndex];
      nextIndex += 1;
      const outcome = await scheduler.read(() => loadText(entry), signal);
      if (outcome.status === "cancelled") return;
      if (outcome.status === "loaded") {
        batch.loaded.push({ outputPath: entry.outputPath, text: outcome.text });
      } else {
        batch.failedOutputPaths.push(entry.outputPath);
      }
      if (
        batch.loaded.length + batch.failedOutputPaths.length
        >= historyPreviewSearchBatchSize
      ) {
        publishBatch();
      } else {
        queueBatchFlush();
      }
    }
  }

  try {
    const workerCount = Math.min(entries.length, historyPreviewSearchConcurrency);
    await Promise.all(Array.from({ length: workerCount }, () => loadNext()));
    publishBatch();
  } finally {
    clearFlushTimeout();
    signal?.removeEventListener("abort", discardBatch);
  }
}

export function createPreviewSearchLoader() {
  const scheduler = createPreviewReadScheduler(historyPreviewSearchConcurrency);
  return {
    load<Entry extends PreviewTextEntry>(options: PreviewSearchLoadOptions<Entry>) {
      return loadPreviewSearchEntriesWithScheduler(options, scheduler);
    },
  };
}

export function loadPreviewSearchEntries<Entry extends PreviewTextEntry>(
  options: PreviewSearchLoadOptions<Entry>,
) {
  return createPreviewSearchLoader().load(options);
}

export function shouldSearchTranscriptBodies(query: string) {
  return query.trim().length >= minTranscriptBodySearchLength;
}

export function previewSearchEntries(entries: TranscriptHistoryEntry[]) {
  return entries.slice(0, maxTranscriptHistoryEntries);
}

export function createPreviewSearchGenerationGuard() {
  let current = 0;
  return {
    begin() {
      current += 1;
      return current;
    },
    isCurrent(generation: number) {
      return generation === current;
    },
  };
}

export function createPreviewTextLoader({
  maxEntries = 8,
  maxCharsPerEntry = 200_000,
  maxTotalChars = 400_000,
}: {
  maxEntries?: number;
  maxCharsPerEntry?: number;
  maxTotalChars?: number;
} = {}) {
  const inFlight = new Map<string, Promise<string>>();
  const failed = new Set<string>();
  let retainedPaths: ReadonlySet<string> | undefined;
  let settled: PreviewTextCache = {};

  return {
    load<Entry extends PreviewTextEntry>(
      entry: Entry,
      cache: PreviewTextCache,
      readText: ((entry: Entry) => Promise<string>) | undefined,
      onLoaded: (outputPath: string, text: string) => void,
    ) {
      const cached = cache[entry.outputPath];
      if (cached !== undefined) return Promise.resolve(cached);
      const settledText = settled[entry.outputPath];
      if (settledText !== undefined) {
        onLoaded(entry.outputPath, settledText);
        return Promise.resolve(settledText);
      }
      if (!readText) return Promise.resolve("");
      if (failed.has(entry.outputPath)) return Promise.reject(new Error("Preview unavailable"));

      const active = inFlight.get(entry.outputPath);
      if (active) {
        return active.then((text) => {
          onLoaded(entry.outputPath, text);
          return text;
        });
      }

      const pending = readText(entry)
        .then((text) => {
          const boundedText = boundTextForCache(text, maxCharsPerEntry);
          if (retainedPaths === undefined || retainedPaths.has(entry.outputPath)) {
            settled = rememberText(
              settled,
              entry.outputPath,
              boundedText,
              maxEntries,
              maxCharsPerEntry,
              maxTotalChars,
            );
          }
          onLoaded(entry.outputPath, boundedText);
          return boundedText;
        }, (error) => {
          if (retainedPaths === undefined || retainedPaths.has(entry.outputPath)) {
            failed.add(entry.outputPath);
          }
          throw error;
        })
        .finally(() => {
          inFlight.delete(entry.outputPath);
        });
      inFlight.set(entry.outputPath, pending);
      return pending;
    },
    prune(paths: ReadonlySet<string>) {
      retainedPaths = paths;
      settled = Object.fromEntries(
        Object.entries(settled).filter(([path]) => paths.has(path)),
      );
      for (const path of failed) {
        if (!paths.has(path)) failed.delete(path);
      }
    },
    retryFailures() {
      failed.clear();
    },
  };
}
