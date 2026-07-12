import { describe, expect, it } from "vitest";

import { pruneTextCache, rememberText, rememberTexts } from "@/lib/text-cache";

describe("text cache", () => {
  it("keeps only the most recent entries", () => {
    let cache: Record<string, string> = {};

    cache = rememberText(cache, "a.txt", "a", 2);
    cache = rememberText(cache, "b.txt", "b", 2);
    cache = rememberText(cache, "c.txt", "c", 2);

    expect(cache).toEqual({ "b.txt": "b", "c.txt": "c" });
  });

  it("refreshes an existing entry without growing the cache", () => {
    let cache = { "a.txt": "old", "b.txt": "b" };

    cache = rememberText(cache, "a.txt", "new", 2);

    expect(cache).toEqual({ "b.txt": "b", "a.txt": "new" });
  });

  it("retains an oversized sentinel so large transcripts are not reread", () => {
    const cache = rememberText({ "a.txt": "a" }, "big.txt", "x".repeat(100), 8, 80);

    expect(cache).toEqual({
      "a.txt": "a",
      "big.txt": "Transcript is too large to preview in the app. Open it from disk instead.",
    });
  });

  it("applies a search-result batch with the same size and entry bounds", () => {
    const cache = rememberTexts(
      { "old.txt": "old" },
      [
        ["a.txt", "a"],
        ["big.txt", "x".repeat(100)],
        ["b.txt", "b"],
      ],
      3,
      80,
    );

    expect(cache).toEqual({
      "a.txt": "a",
      "big.txt": "Transcript is too large to preview in the app. Open it from disk instead.",
      "b.txt": "b",
    });
  });

  it("evicts oldest previews to enforce the aggregate character budget", () => {
    const cache = rememberTexts(
      {},
      [["a.txt", "1234"], ["b.txt", "5678"], ["c.txt", "90"]],
      10,
      10,
      6,
    );

    expect(cache).toEqual({ "b.txt": "5678", "c.txt": "90" });
    expect(Object.values(cache).reduce((total, text) => total + text.length, 0)).toBe(6);
  });

  it("bounds oversized values already present in the input cache", () => {
    const cache = rememberTexts({ "huge.txt": "x".repeat(100) }, [], 10, 80, 100);

    expect(cache["huge.txt"]).toBe(
      "Transcript is too large to preview in the app. Open it from disk instead.",
    );
  });

  it("never lets the oversized sentinel exceed the per-entry budget", () => {
    const cache = rememberText({}, "big.txt", "12345", 8, 4, 100);

    expect(cache).toEqual({ "big.txt": "Tran" });
  });

  it("prunes stale paths before new results can evict current history", () => {
    const currentPaths = new Set(["current-a.txt", "current-b.txt"]);
    const pruned = pruneTextCache(
      {
        "current-a.txt": "current a",
        "stale-a.txt": "stale a",
        "stale-b.txt": "stale b",
      },
      currentPaths,
    );

    const cache = rememberTexts(
      pruned,
      [["current-b.txt", "current b"]],
      currentPaths.size,
    );

    expect(cache).toEqual({
      "current-a.txt": "current a",
      "current-b.txt": "current b",
    });
  });
});
