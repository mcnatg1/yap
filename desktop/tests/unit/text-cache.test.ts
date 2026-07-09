import { describe, expect, it } from "vitest";

import { rememberText } from "@/lib/text-cache";

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
    const cache = rememberText({ "a.txt": "a" }, "big.txt", "12345", 8, 4);

    expect(cache).toEqual({
      "a.txt": "a",
      "big.txt": "Transcript is too large to preview in the app. Open it from disk instead.",
    });
  });
});
