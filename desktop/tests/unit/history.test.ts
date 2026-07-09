import { describe, expect, it } from "vitest";

import { canDeleteTranscriptHistoryEntry, hideTranscriptHistory, normalizeHiddenTranscriptHistory, readTranscriptHistory } from "@/history";

describe("transcript history storage", () => {
  it("dedupes hidden transcript paths", () => {
    expect(hideTranscriptHistory(["a.txt"], "a.txt")).toEqual(["a.txt"]);
    expect(normalizeHiddenTranscriptHistory(["a.txt", 42, "b.txt", "a.txt"])).toEqual([
      "a.txt",
      "b.txt",
    ]);
  });

  it("preserves warning metadata for partial live saves", () => {
    const storage = {
      getItem: () => JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-123",
        outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
        sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
        warning: "Live audio could not be saved. Transcript was saved.",
      }]),
      setItem: () => undefined,
    };

    expect(readTranscriptHistory(storage)[0].warning).toBe("Live audio could not be saved. Transcript was saved.");
  });

  it("only exposes delete for Yap-owned live history entries", () => {
    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav",
    })).toBe(true);

    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "meeting-notes",
      outputPath: "C:\\Users\\me\\Documents\\meeting-notes.txt",
      sourcePath: "C:\\Users\\me\\Downloads\\meeting.mp3",
    })).toBe(false);

    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\Documents\\live-123.txt",
      sourcePath: "C:\\Users\\me\\Documents\\live-123.wav",
    })).toBe(false);

    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-999.wav",
    })).toBe(false);
  });
});
