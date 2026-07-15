import { describe, expect, it } from "vitest";

import {
  filterHiddenTranscriptHistory,
  hideTranscriptHistory,
  normalizeHiddenTranscriptHistory,
  recordVisibleTranscriptHistoryEntries,
  transcriptPathIdentity,
} from "@/history-model";

describe("transcript history model", () => {
  it("dedupes hidden transcript paths", () => {
    expect(hideTranscriptHistory(["a.txt"], "a.txt")).toEqual(["a.txt"]);
    expect(normalizeHiddenTranscriptHistory(["a.txt", 42, "b.txt", "a.txt"])).toEqual([
      "a.txt",
      "b.txt",
    ]);
  });

  it("matches Windows transcript paths across case, separators, and verbatim prefixes", () => {
    const canonical = "C:\\Users\\Me\\AppData\\Local\\Yap\\live-recordings\\live-1.txt";
    const slashCaseVariant = "c:/users/me/AppData/Local/YAP/live-recordings/./live-1.txt";
    const verbatimVariant = "\\\\?\\C:\\Users\\Me\\AppData\\Local\\Yap\\live-recordings\\live-1.txt";

    expect(transcriptPathIdentity(slashCaseVariant)).toBe(transcriptPathIdentity(canonical));
    expect(transcriptPathIdentity(verbatimVariant)).toBe(transcriptPathIdentity(canonical));
    expect(normalizeHiddenTranscriptHistory([
      canonical,
      slashCaseVariant,
      verbatimVariant,
    ])).toEqual([canonical]);
    expect(filterHiddenTranscriptHistory([{
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-1",
      outputPath: slashCaseVariant,
      sourcePath: slashCaseVariant,
    }], [canonical])).toEqual([]);
    expect(transcriptPathIdentity("C:\\..\\..\\Yap\\live-1.txt"))
      .toBe(transcriptPathIdentity("C:\\Yap\\live-1.txt"));
    expect(transcriptPathIdentity("\\\\server\\share\\..\\Yap\\live-1.txt"))
      .toBe(transcriptPathIdentity("\\\\server\\share\\Yap\\live-1.txt"));
  });

  it("records only visible incoming history entries", () => {
    const hidden = {
      createdAt: "2026-01-03T00:00:00.000Z",
      name: "hidden",
      outputPath: "hidden.txt",
      sourcePath: "hidden.wav",
    };
    const visible = {
      createdAt: "2026-01-04T00:00:00.000Z",
      name: "visible",
      outputPath: "visible.txt",
      sourcePath: "visible.wav",
    };

    expect(recordVisibleTranscriptHistoryEntries([], [hidden, visible], ["hidden.txt"]))
      .toEqual([visible]);
  });
});
