import { describe, expect, it } from "vitest";

import { maxTranscriptHistoryEntries, type TranscriptHistoryEntry } from "@/history";
import {
  createPreviewSearchGenerationGuard,
  createPreviewTextLoader,
  previewSearchEntries,
  shouldSearchTranscriptBodies,
} from "@/lib/history-preview-loader";

describe("history preview loader", () => {
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
