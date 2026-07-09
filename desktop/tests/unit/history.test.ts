import { describe, expect, it } from "vitest";

import {
  canDeleteTranscriptHistoryEntry,
  filterHiddenTranscriptHistory,
  historyEntryPlaybackPath,
  hideTranscriptHistory,
  maxTranscriptHistoryEntries,
  normalizeHiddenTranscriptHistory,
  readTranscriptHistory,
  readVisibleTranscriptHistory,
  recordVisibleTranscriptHistoryEntries,
  savedSessionToTranscriptHistoryEntry,
} from "@/history";

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

  it("bounds persisted transcript history to recent entries", () => {
    const entries = Array.from({ length: maxTranscriptHistoryEntries + 5 }, (_, index) => ({
      createdAt: new Date(Date.UTC(2026, 0, index + 1)).toISOString(),
      name: `live-${index}`,
      outputPath: `C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-${index}.txt`,
      sourcePath: `C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-${index}.wav`,
    }));
    const storage = {
      getItem: () => JSON.stringify(entries),
      setItem: () => undefined,
    };

    const history = readTranscriptHistory(storage);

    expect(history).toHaveLength(maxTranscriptHistoryEntries);
    expect(history[0].name).toBe(`live-${maxTranscriptHistoryEntries + 4}`);
    expect(history.at(-1)?.name).toBe("live-5");
  });

  it("keeps hidden transcripts out of history after reload", () => {
    const hidden = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-hidden.txt";
    const visible = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-visible.txt";
    const storage = {
      getItem: (key: string) => {
        if (key === "yap.hiddenTranscriptHistory.v1") return JSON.stringify([hidden]);
        if (key === "yap.transcriptHistory.v1") {
          return JSON.stringify([
            {
              createdAt: "2026-01-02T00:00:00.000Z",
              name: "live-visible",
              outputPath: visible,
              sourcePath: visible,
            },
            {
              createdAt: "2026-01-01T00:00:00.000Z",
              name: "live-hidden",
              outputPath: hidden,
              sourcePath: hidden,
            },
          ]);
        }
        return null;
      },
      setItem: () => undefined,
    };

    expect(readVisibleTranscriptHistory(storage).map((entry) => entry.outputPath)).toEqual([visible]);
    expect(filterHiddenTranscriptHistory(readTranscriptHistory(storage), [hidden])).toHaveLength(1);
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

    const next = recordVisibleTranscriptHistoryEntries([], [hidden, visible], ["hidden.txt"]);

    expect(next).toEqual([visible]);
  });

  it("projects saved live sessions into history entries", () => {
    const entry = savedSessionToTranscriptHistoryEntry({
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-1",
      outputPath: "live-1.txt",
      sourcePath: "live-1.wav",
      warning: null,
    });

    expect(entry).toEqual({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-1",
      outputPath: "live-1.txt",
      sourcePath: "live-1.wav",
      warning: undefined,
    });
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

  it("only exposes playback for matching Yap-owned live audio", () => {
    const outputPath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt";
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav";

    expect(historyEntryPlaybackPath({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath,
      sourcePath,
    })).toBe(sourcePath);

    expect(historyEntryPlaybackPath({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath,
      sourcePath: outputPath,
    })).toBeUndefined();

    expect(historyEntryPlaybackPath({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "meeting-notes",
      outputPath: "C:\\Users\\me\\Documents\\meeting-notes.txt",
      sourcePath: "C:\\Users\\me\\Downloads\\meeting.mp3",
    })).toBeUndefined();
  });
});
