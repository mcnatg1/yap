import { describe, expect, it } from "vitest";

import {
  filterLegacyHiddenTranscriptHistory,
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

  it("bounds hidden transcript paths to the newest history window", () => {
    const hidden = Array.from({ length: 501 }, (_, index) => `hidden-${index}.txt`);

    expect(normalizeHiddenTranscriptHistory(hidden)).toEqual(hidden.slice(0, 500));
    expect(hideTranscriptHistory(hidden.slice(0, 500), "newest.txt")).toEqual([
      "newest.txt",
      ...hidden.slice(0, 499),
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

  it("keeps native catalog rows outside the legacy compatibility tombstones", () => {
    const legacy = {
      createdAt: "2026-01-03T00:00:00.000Z",
      name: "legacy",
      outputPath: "same.txt",
      sourcePath: "legacy.wav",
    };
    const native = {
      ...legacy,
      name: "native",
      origin: "remote" as const,
      sessionId: "remote-1",
    };

    expect(filterLegacyHiddenTranscriptHistory([legacy, native], ["same.txt"]))
      .toEqual([native]);
  });
});
