import { describe, expect, it } from "vitest";

import {
  filterHiddenTranscriptHistory,
  maxTranscriptHistoryEntries,
} from "@/history-model";
import {
  compactHiddenTranscriptHistory,
  readHiddenTranscriptHistory,
  readTranscriptHistory,
  readVisibleTranscriptHistory,
  removeMigratedHiddenTranscriptHistory,
} from "@/history-storage";

describe("transcript history persistence", () => {
  it("compacts an oversized legacy tombstone value on disk", () => {
    const values = new Map<string, string>([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify(
        Array.from({ length: 501 }, (_, index) => `hidden-${index}.txt`),
      )],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    compactHiddenTranscriptHistory(storage);

    expect(JSON.parse(values.get("yap.hiddenTranscriptHistory.v1") ?? "[]"))
      .toHaveLength(500);
  });

  it("removes migrated native tombstones without dropping concurrent legacy entries", () => {
    const values = new Map<string, string>([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([
        "new-legacy.txt",
        "C:/Yap/native.txt",
        "old-legacy.txt",
      ])],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    removeMigratedHiddenTranscriptHistory(["c:\\yap\\native.txt"], storage);

    expect(readHiddenTranscriptHistory(storage)).toEqual([
      "new-legacy.txt",
      "old-legacy.txt",
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

    expect(readTranscriptHistory(storage)[0].warning)
      .toBe("Live audio could not be saved. Transcript was saved.");
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
              captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-visible.commit.json",
              createdAt: "2026-01-02T00:00:00.000Z",
              name: "live-visible",
              outputPath: visible,
              sourcePath: visible,
            },
            {
              captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-hidden.commit.json",
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

  it("does not expose strict pre-release localStorage rows outside the default Yap path", () => {
    const storage = {
      getItem: () => JSON.stringify([
        {
          createdAt: "2026-01-01T00:00:00.000Z",
          name: "live-1720656000000",
          outputPath: "D:\\custom-recordings\\live-1720656000000.txt",
          sourcePath: "D:\\custom-recordings\\live-1720656000000.wav",
        },
        {
          createdAt: "2026-01-02T00:00:00.000Z",
          name: "live-1720656000001-2",
          outputPath: "relative-recordings/live-1720656000001-2.txt",
          sourcePath: "relative-recordings/live-1720656000001-2.wav",
        },
        {
          createdAt: "2026-01-03T00:00:00.000Z",
          name: "live-1720656000002",
          outputPath: "imports/interview-transcript.txt",
          sourcePath: "imports/interview.wav",
        },
      ]),
      setItem: () => undefined,
    };

    expect(readVisibleTranscriptHistory(storage)).toEqual([
      {
        createdAt: "2026-01-03T00:00:00.000Z",
        name: "live-1720656000002",
        outputPath: "imports/interview-transcript.txt",
        sourcePath: "imports/interview.wav",
      },
    ]);
  });

  it("never revives a timestamp pre-release row with a fabricated commit path", () => {
    const storage = {
      getItem: () => JSON.stringify([{
        captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-1720656000000.commit.json",
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-1720656000000",
        outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-1720656000000.txt",
        sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-1720656000000.wav",
      }]),
      setItem: () => undefined,
    };

    expect(readVisibleTranscriptHistory(storage)).toEqual([]);
  });
});
