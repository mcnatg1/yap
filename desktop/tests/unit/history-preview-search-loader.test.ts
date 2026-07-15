import { describe, expect, it, vi } from "vitest";

import { maxTranscriptHistoryEntries } from "@/history-model";
import {
  createPreviewSearchLoader,
  historyPreviewSearchBatchSize,
  historyPreviewSearchConcurrency,
  historyPreviewSearchFlushDelayMs,
  loadPreviewSearchEntries,
} from "@/lib/history-preview-loader";

describe("history preview search scheduler", () => {
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
});
