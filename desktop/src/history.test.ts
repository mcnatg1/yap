import { describe, expect, it } from "vitest";

import { hideTranscriptHistory, normalizeHiddenTranscriptHistory } from "@/history";

describe("transcript history storage", () => {
  it("dedupes hidden transcript paths", () => {
    expect(hideTranscriptHistory(["a.txt"], "a.txt")).toEqual(["a.txt"]);
    expect(normalizeHiddenTranscriptHistory(["a.txt", 42, "b.txt", "a.txt"])).toEqual([
      "a.txt",
      "b.txt",
    ]);
  });
});
