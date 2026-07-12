import { describe, expect, it, vi } from "vitest";

import { maxTranscriptHistoryEntries, type TranscriptHistoryEntry } from "@/history";
import {
  createPreviewSearchLoader,
  createPreviewSearchGenerationGuard,
  createPreviewTextLoader,
  historyPreviewSearchBatchSize,
  historyPreviewSearchConcurrency,
  historyPreviewSearchFlushDelayMs,
  loadPreviewSearchEntries,
  mergePreviewSearchFailures,
  previewSearchEntries,
  shouldSearchTranscriptBodies,
} from "@/lib/history-preview-loader";

describe("history preview loader", () => {
  it("caps concurrent transcript body reads", async () => {
    const entries = Array.from(
      { length: historyPreviewSearchConcurrency + 3 },
      (_, index) => ({ outputPath: `C:\\recordings\\live-${index}.txt` }),
    );
    const releases = entries.map(() => Promise.withResolvers<string>());
    let activeReads = 0;
    let maxActiveReads = 0;

    const search = loadPreviewSearchEntries({
      entries,
      loadText: (entry) => {
        activeReads += 1;
        maxActiveReads = Math.max(maxActiveReads, activeReads);
        const index = entries.indexOf(entry);
        return releases[index].promise.finally(() => {
          activeReads -= 1;
        });
      },
      onBatch: () => undefined,
    });

    expect(activeReads).toBe(historyPreviewSearchConcurrency);

    releases.forEach((release, index) => release.resolve(`text ${index}`));
    await search;

    expect(maxActiveReads).toBe(historyPreviewSearchConcurrency);
  });

  it("retains native-read slots and suppresses stale batches after cancellation", async () => {
    const staleEntries = Array.from(
      { length: historyPreviewSearchConcurrency },
      (_, index) => ({ outputPath: `stale-${index}.txt` }),
    );
    const currentEntries = Array.from(
      { length: historyPreviewSearchConcurrency },
      (_, index) => ({ outputPath: `current-${index}.txt` }),
    );
    const staleReleases = staleEntries.map(() => Promise.withResolvers<string>());
    const currentReleases = currentEntries.map(() => Promise.withResolvers<string>());
    const controller = new AbortController();
    const publications: string[] = [];
    const loader = createPreviewSearchLoader();
    let activeReads = 0;
    let maxActiveReads = 0;
    let nativeReads = 0;

    const loadText = (entry: { outputPath: string }) => {
      const staleIndex = staleEntries.indexOf(entry);
      const pending = staleIndex >= 0
        ? staleReleases[staleIndex].promise
        : currentReleases[currentEntries.indexOf(entry)].promise;
      activeReads += 1;
      nativeReads += 1;
      maxActiveReads = Math.max(maxActiveReads, activeReads);
      return pending.finally(() => {
        activeReads -= 1;
      });
    };

    const staleSearch = loader.load({
      entries: staleEntries,
      loadText,
      onBatch: (batch) => {
        publications.push(...batch.loaded.map(({ outputPath }) => `stale:${outputPath}`));
      },
      signal: controller.signal,
    });

    expect(nativeReads).toBe(historyPreviewSearchConcurrency);
    controller.abort();
    const cancellationTimeout = Promise.withResolvers<boolean>();
    const timeout = setTimeout(() => cancellationTimeout.resolve(false), 50);
    expect(await Promise.race([
      staleSearch.then(() => true),
      cancellationTimeout.promise,
    ])).toBe(true);
    clearTimeout(timeout);

    const currentSearch = loader.load({
      entries: currentEntries,
      loadText,
      onBatch: (batch) => {
        publications.push(...batch.loaded.map(({ outputPath }) => `current:${outputPath}`));
      },
    });

    expect(activeReads).toBe(historyPreviewSearchConcurrency);
    expect(nativeReads).toBe(historyPreviewSearchConcurrency);

    staleReleases.forEach((release) => release.resolve("stale text"));
    await vi.waitFor(() => expect(nativeReads).toBe(historyPreviewSearchConcurrency * 2));
    currentReleases.forEach((release) => release.resolve("current text"));
    await currentSearch;

    expect(maxActiveReads).toBe(historyPreviewSearchConcurrency);
    expect(publications).toEqual(
      currentEntries.map(({ outputPath }) => `current:${outputPath}`),
    );
  });

  it("flushes partial successes promptly when another read hangs", async () => {
    vi.useFakeTimers();
    const controller = new AbortController();
    const hangingRead = Promise.withResolvers<string>();
    const batches: string[][] = [];
    const loader = createPreviewSearchLoader();

    try {
      const search = loader.load({
        entries: [
          { outputPath: "ready.txt" },
          { outputPath: "hanging.txt" },
        ],
        loadText: (entry) => entry.outputPath === "ready.txt"
          ? Promise.resolve("ready text")
          : hangingRead.promise,
        onBatch: (batch) => {
          batches.push(batch.loaded.map(({ outputPath }) => outputPath));
        },
        signal: controller.signal,
      });

      await vi.advanceTimersByTimeAsync(historyPreviewSearchFlushDelayMs - 1);
      expect(batches).toEqual([]);

      await vi.advanceTimersByTimeAsync(1);
      expect(batches).toEqual([["ready.txt"]]);

      controller.abort();
      await search;
      hangingRead.resolve("late text");
      await hangingRead.promise;
    } finally {
      vi.useRealTimers();
    }
  });

  it("publishes a full history search in a bounded number of state batches", async () => {
    const entries = Array.from(
      { length: maxTranscriptHistoryEntries },
      (_, index) => ({ outputPath: `C:\\recordings\\live-${index}.txt` }),
    );
    const publishedBatchSizes: number[] = [];

    await loadPreviewSearchEntries({
      entries,
      loadText: async (entry) => `text for ${entry.outputPath}`,
      onBatch: (batch) => {
        publishedBatchSizes.push(batch.loaded.length + batch.failedOutputPaths.length);
      },
    });

    expect(publishedBatchSizes).toHaveLength(
      Math.ceil(maxTranscriptHistoryEntries / historyPreviewSearchBatchSize),
    );
    expect(Math.max(...publishedBatchSizes)).toBe(historyPreviewSearchBatchSize);
    expect(publishedBatchSizes.reduce((total, size) => total + size, 0))
      .toBe(maxTranscriptHistoryEntries);
  });

  it("clears successful failure markers across query changes", () => {
    const failures = mergePreviewSearchFailures({
      current: {
        paths: new Set(["recovered.txt", "old-query-only.txt"]),
      },
      failedOutputPaths: [],
      loadedOutputPaths: ["recovered.txt"],
      visibleOutputPaths: new Set([
        "recovered.txt",
        "old-query-only.txt",
      ]),
    });

    expect(failures).toEqual({
      paths: new Set(["old-query-only.txt"]),
    });
  });

  it("reuses a stale generation read when the current generation requests the same path", async () => {
    const loader = createPreviewTextLoader();
    const release = Promise.withResolvers<string>();
    const entry = { outputPath: "shared.txt" };
    let reads = 0;
    const readText = () => {
      reads += 1;
      return release.promise;
    };

    const stale = loader.load(entry, {}, readText, () => undefined);
    release.resolve("settled stale text");
    await stale;
    const current = await loader.load(entry, {}, readText, () => undefined);

    expect(current).toBe("settled stale text");
    expect(reads).toBe(1);
  });

  it("remembers unreadable paths until an explicit retry", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "missing.txt" };
    let reads = 0;
    const readText = async () => {
      reads += 1;
      throw new Error("missing");
    };

    await expect(loader.load(entry, {}, readText, () => undefined)).rejects.toThrow("missing");
    await expect(loader.load(entry, {}, readText, () => undefined)).rejects.toThrow();
    expect(reads).toBe(1);

    loader.retryFailures();
    await expect(loader.load(entry, {}, readText, () => undefined)).rejects.toThrow("missing");
    expect(reads).toBe(2);
  });

  it("prunes settled paths and does not retain late reads for removed entries", async () => {
    const loader = createPreviewTextLoader();
    const release = Promise.withResolvers<string>();
    const entry = { outputPath: "removed.txt" };
    let reads = 0;
    const readText = () => {
      reads += 1;
      return reads === 1 ? release.promise : Promise.resolve("fresh text");
    };

    const late = loader.load(entry, {}, readText, () => undefined);
    loader.prune(new Set());
    release.resolve("late text");
    await late;
    loader.prune(new Set([entry.outputPath]));

    expect(await loader.load(entry, {}, readText, () => undefined)).toBe("fresh text");
    expect(reads).toBe(2);
  });

  it("invalidates late preview results when a newer search begins", () => {
    const guard = createPreviewSearchGenerationGuard();
    const first = guard.begin();
    const second = guard.begin();

    expect(guard.isCurrent(first)).toBe(false);
    expect(guard.isCurrent(second)).toBe(true);
  });

  it("dedupes concurrent reads by output path", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "C:\\recordings\\live-1.txt" };
    let reads = 0;
    const loaded: Record<string, string> = {};
    const readText = async () => {
      reads += 1;
      return "hello";
    };

    const [first, second] = await Promise.all([
      loader.load(entry, {}, readText, (path, text) => {
        loaded[path] = text;
      }),
      loader.load(entry, {}, readText, (path, text) => {
        loaded[path] = text;
      }),
    ]);

    expect(first).toBe("hello");
    expect(second).toBe("hello");
    expect(reads).toBe(1);
    expect(loaded[entry.outputPath]).toBe("hello");
  });

  it("notifies each concurrent caller when a shared read completes", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "C:\\recordings\\live-shared.txt" };
    const loaded: string[] = [];

    const [first, second] = await Promise.all([
      loader.load(entry, {}, async () => "shared text", (path, text) => {
        loaded.push(`first:${path}:${text}`);
      }),
      loader.load(entry, {}, async () => "unexpected", (path, text) => {
        loaded.push(`second:${path}:${text}`);
      }),
    ]);

    expect(first).toBe("shared text");
    expect(second).toBe("shared text");
    expect(loaded).toEqual([
      `first:${entry.outputPath}:shared text`,
      `second:${entry.outputPath}:shared text`,
    ]);
  });

  it("returns cached empty previews without reading again", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "C:\\recordings\\live-empty.txt" };
    let reads = 0;

    const text = await loader.load(entry, { [entry.outputPath]: "" }, async () => {
      reads += 1;
      return "unexpected";
    }, () => undefined);

    expect(text).toBe("");
    expect(reads).toBe(0);
  });

  it("bounds oversized native results before returning them to search callers", async () => {
    const loader = createPreviewTextLoader({
      maxCharsPerEntry: 80,
      maxEntries: 2,
      maxTotalChars: 100,
    });
    const loaded: string[] = [];

    const text = await loader.load(
      { outputPath: "huge.txt" },
      {},
      async () => "x".repeat(100_000),
      (_path, preview) => loaded.push(preview),
    );

    expect(text).toBe(
      "Transcript is too large to preview in the app. Open it from disk instead.",
    );
    expect(loaded).toEqual([text]);
  });

  it("rejects failed reads so the row can show its fallback state", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "C:\\recordings\\missing.txt" };

    await expect(loader.load(entry, {}, async () => {
      throw new Error("missing");
    }, () => undefined)).rejects.toThrow("missing");
  });

  it("does not search transcript bodies for one-character queries", () => {
    expect(shouldSearchTranscriptBodies("a")).toBe(false);
    expect(shouldSearchTranscriptBodies("  a ")).toBe(false);
    expect(shouldSearchTranscriptBodies("ab")).toBe(true);
  });

  it("does not index more than the bounded history cap", () => {
    const entries = Array.from({ length: maxTranscriptHistoryEntries + 5 }, (_, index): TranscriptHistoryEntry => ({
      createdAt: new Date(index).toISOString(),
      name: `live-${index}`,
      outputPath: `C:\\recordings\\live-${index}.txt`,
      sourcePath: `C:\\recordings\\live-${index}.wav`,
    }));

    const searchable = previewSearchEntries(entries);

    expect(searchable).toHaveLength(maxTranscriptHistoryEntries);
    expect(searchable[0]).toBe(entries[0]);
    expect(searchable.at(-1)).toBe(entries[maxTranscriptHistoryEntries - 1]);
  });
});
