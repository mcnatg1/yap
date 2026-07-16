import { describe, expect, it } from "vitest";

import { maxTranscriptHistoryEntries, type TranscriptHistoryEntry } from "@/history-model";
import {
  createPreviewSearchGenerationGuard,
  createPreviewTextLoader,
  mergePreviewSearchFailures,
  previewSearchEntries,
  shouldSearchTranscriptBodies,
} from "@/lib/history-preview-loader";

describe("history preview loader", () => {
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
