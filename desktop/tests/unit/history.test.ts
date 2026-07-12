import { describe, expect, it } from "vitest";

import {
  canDeleteTranscriptHistoryEntry,
  filterHiddenTranscriptHistory,
  historyEntryPlaybackPath,
  hideTranscriptHistory,
  maxTranscriptHistoryEntries,
  normalizeHiddenTranscriptHistory,
  pruneMissingHiddenTranscriptHistory,
  readHiddenTranscriptHistory,
  readTranscriptHistory,
  readVisibleTranscriptHistory,
  recoverableLiveSessionActionIdentity,
  reconcileNativeTranscriptHistoryEntries,
  recordVisibleTranscriptHistoryEntries,
  savedSessionToTranscriptHistoryEntry,
  savedLiveSessionActionIdentity,
  transcriptPathIdentity,
  writeTranscriptHistory,
  type OwnedLiveTranscriptPathResolution,
} from "@/history";

function missingResolution(requestedPath: string): OwnedLiveTranscriptPathResolution {
  return { canonicalPath: null, missing: true, requestedPath };
}

describe("transcript history storage", () => {
  it("replaces only Rust-managed rows after a native reconciliation", () => {
    const nativeGone = {
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-gone.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-gone",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-gone.wav",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-gone.wav",
    };
    const legacy = {
      createdAt: "2025-12-01T00:00:00.000Z",
      name: "live-legacy",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-legacy.txt",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-legacy.txt",
    };
    const imported = {
      createdAt: "2025-11-01T00:00:00.000Z",
      name: "imported",
      outputPath: "C:\\Users\\me\\Documents\\imported.txt",
      sourcePath: "C:\\Users\\me\\Documents\\imported.txt",
    };
    const incoming = {
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-current.commit.json",
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "live-current",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-current.wav",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-current.wav",
    };

    expect(reconcileNativeTranscriptHistoryEntries(
      [nativeGone, legacy, imported],
      [incoming],
      [],
    ).map((entry) => entry.name)).toEqual(["live-current", "live-legacy", "imported"]);
  });

  it("keeps hidden native tombstones hidden during a refresh", () => {
    const entry = {
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-hidden.commit.json",
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "live-hidden",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-hidden.wav",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-hidden.wav",
    };

    expect(reconcileNativeTranscriptHistoryEntries([], [entry], [entry.outputPath])).toEqual([]);
  });
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

  it("prunes only Rust-confirmed hidden outputs", async () => {
    const values = new Map([["yap.hiddenTranscriptHistory.v1", JSON.stringify(["existing", "missing", "external"])]]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    const next = await pruneMissingHiddenTranscriptHistory(async () => [
      missingResolution("missing"),
      missingResolution("not-requested"),
    ], storage);

    expect(next).toEqual(["existing", "external"]);
    expect(readHiddenTranscriptHistory(storage)).toEqual(["existing", "external"]);
  });

  it("preserves tombstones added while Rust authorization is pending", async () => {
    let raw = JSON.stringify(["missing", "existing"]);
    let resolveAuthorization!: (paths: OwnedLiveTranscriptPathResolution[]) => void;
    const storage = {
      getItem: () => raw,
      setItem: (_key: string, value: string) => { raw = value; },
    };
    const authorization = new Promise<OwnedLiveTranscriptPathResolution[]>((resolve) => {
      resolveAuthorization = resolve;
    });

    const pruning = pruneMissingHiddenTranscriptHistory(() => authorization, storage);
    await Promise.resolve();
    raw = JSON.stringify(["newly-hidden", "missing", "existing"]);
    resolveAuthorization([missingResolution("missing")]);
    await pruning;

    expect(readHiddenTranscriptHistory(storage)).toEqual(["newly-hidden", "existing"]);
  });

  it("migrates an existing alias tombstone to Rust's canonical path", async () => {
    const alias = "C:\\Alias\\live-1.txt";
    const canonical = "D:\\Real\\live-1.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([alias])],
      ["yap.transcriptHistory.v1", JSON.stringify([
        {
          createdAt: "2026-01-02T00:00:00.000Z",
          name: "live-1",
          outputPath: canonical,
          sourcePath: canonical,
        },
        {
          createdAt: "2026-01-01T00:00:00.000Z",
          name: "live-1-alias",
          outputPath: alias,
          sourcePath: alias,
        },
      ])],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    await pruneMissingHiddenTranscriptHistory(async () => [{
      canonicalPath: canonical,
      missing: false,
      requestedPath: alias,
    }], storage);

    expect(readHiddenTranscriptHistory(storage)).toEqual([canonical]);
    expect(readTranscriptHistory(storage).map((entry) => entry.outputPath)).toEqual([canonical]);
    expect(readVisibleTranscriptHistory(storage)).toEqual([]);
  });

  it("removes canonical history when a missing alias resolves after deletion", async () => {
    const alias = "C:\\Alias\\live-2.txt";
    const canonical = "D:\\Real\\live-2.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([alias])],
      ["yap.transcriptHistory.v1", JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-2",
        outputPath: canonical,
        sourcePath: canonical,
      }])],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    await pruneMissingHiddenTranscriptHistory(async () => [{
      canonicalPath: canonical,
      missing: true,
      requestedPath: alias,
    }], storage);

    expect(readTranscriptHistory(storage)).toEqual([]);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
  });

  it("protects a canonical tombstone before removing stale alias metadata", async () => {
    const alias = "C:\\Alias\\live-3.txt";
    const canonical = "D:\\Real\\live-3.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([alias])],
      ["yap.transcriptHistory.v1", JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-3-alias",
        outputPath: alias,
        sourcePath: alias,
      }])],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        if (key === "yap.transcriptHistory.v1") throw new Error("quota full");
        values.set(key, value);
      },
    };

    await expect(pruneMissingHiddenTranscriptHistory(async () => [{
      canonicalPath: canonical,
      missing: false,
      requestedPath: alias,
    }], storage)).rejects.toThrow("quota full");

    expect(readHiddenTranscriptHistory(storage)).toEqual([alias, canonical]);
  });

  it("removes stale history metadata before pruning its tombstone", async () => {
    const missing = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-101.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([missing])],
      ["yap.transcriptHistory.v1", JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-101",
        outputPath: missing,
        sourcePath: missing,
      }])],
    ]);
    const writes: string[] = [];
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        writes.push(key);
        values.set(key, value);
      },
    };

    await pruneMissingHiddenTranscriptHistory(async () => [missingResolution(missing)], storage);

    expect(readTranscriptHistory(storage)).toEqual([]);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
    expect(writes).toEqual([
      "yap.transcriptHistory.v1",
      "yap.hiddenTranscriptHistory.v1",
    ]);
  });

  it("keeps the tombstone when stale history cleanup cannot be saved", async () => {
    const missing = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-102.txt";
    const history = JSON.stringify([{
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-102",
      outputPath: missing,
      sourcePath: missing,
    }]);
    const hidden = JSON.stringify([missing]);
    const storage = {
      getItem: (key: string) => key === "yap.transcriptHistory.v1" ? history : hidden,
      setItem: (key: string) => {
        if (key === "yap.transcriptHistory.v1") throw new Error("quota full");
        throw new Error("tombstone must not be written");
      },
    };

    await expect(pruneMissingHiddenTranscriptHistory(async () => [missingResolution(missing)], storage))
      .rejects.toThrow("quota full");
    expect(readHiddenTranscriptHistory(storage)).toEqual([missing]);
  });

  it("removes a stale rescan row that resolves while pruning is pending", async () => {
    const missing = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-103.txt";
    const values = new Map([["yap.hiddenTranscriptHistory.v1", JSON.stringify([missing])]]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };
    let resolveAuthorization!: (paths: OwnedLiveTranscriptPathResolution[]) => void;
    const authorization = new Promise<OwnedLiveTranscriptPathResolution[]>((resolve) => {
      resolveAuthorization = resolve;
    });

    const pruning = pruneMissingHiddenTranscriptHistory(() => authorization, storage);
    await Promise.resolve();
    writeTranscriptHistory([{
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-103",
      outputPath: missing,
      sourcePath: missing,
    }], storage);
    resolveAuthorization([missingResolution(missing)]);
    await pruning;

    expect(readTranscriptHistory(storage)).toEqual([]);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
  });

  it("does not write tombstones when an authorization batch fails", async () => {
    const original = JSON.stringify(Array.from({ length: 201 }, (_, index) => `live-${index}.txt`));
    let raw = original;
    let writes = 0;
    const storage = {
      getItem: () => raw,
      setItem: (_key: string, value: string) => {
        writes += 1;
        raw = value;
      },
    };
    let calls = 0;

    await expect(pruneMissingHiddenTranscriptHistory(async () => {
      calls += 1;
      if (calls === 2) throw new Error("native unavailable");
      return [missingResolution("live-0.txt")];
    }, storage)).rejects.toThrow("native unavailable");

    expect(calls).toBe(2);
    expect(writes).toBe(0);
    expect(raw).toBe(original);
  });

  it("batches tombstone authorization and commits once", async () => {
    const paths = Array.from({ length: 201 }, (_, index) => `live-${index}.txt`);
    let raw = JSON.stringify(paths);
    let writes = 0;
    const batchSizes: number[] = [];
    const storage = {
      getItem: () => raw,
      setItem: (_key: string, value: string) => {
        writes += 1;
        raw = value;
      },
    };

    await pruneMissingHiddenTranscriptHistory(async (batch) => {
      batchSizes.push(batch.length);
      return batch.map(missingResolution);
    }, storage);

    expect(batchSizes).toEqual([200, 1]);
    expect(writes).toBe(1);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
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
      sessionId: "1",
      sourcePath: "live-1.wav",
      warning: null,
    });

    expect(entry).toEqual({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-1",
      outputPath: "live-1.txt",
      sessionId: "1",
      sourcePath: "live-1.wav",
      warning: undefined,
    });
  });

  it("preserves old localStorage rows while retaining native recovery metadata", () => {
    const storage = {
      getItem: () => JSON.stringify([
        {
          createdAt: "2026-01-01T00:00:00.000Z",
          name: "legacy-live",
          outputPath: "legacy-live.txt",
          sourcePath: "legacy-live.wav",
        },
      ]),
      setItem: () => undefined,
    };

    expect(readTranscriptHistory(storage)).toEqual([
      {
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "legacy-live",
        outputPath: "legacy-live.txt",
        sourcePath: "legacy-live.wav",
      },
    ]);

    const entry = savedSessionToTranscriptHistoryEntry({
      createdAtMs: Date.UTC(2026, 0, 2),
      name: "live-recoverable",
      outputPath: "live-recoverable.wav.part",
      sessionId: "recoverable",
      sourcePath: "live-recoverable.wav.part",
      warning: null,
      captureCommitPath: undefined,
      recoveryState: "recoverable",
    } as SavedTranscriptSession & {
      captureCommitPath?: string;
      recoveryState?: "recoverable";
    });

    expect(entry.recoveryState).toBe("recoverable");
    expect(entry.captureCommitPath).toBeUndefined();
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

  it("only exposes delete for Yap-owned live history entries", () => {
    expect(canDeleteTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sessionId: "123",
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
      sessionId: "123",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-999.wav",
    })).toBe(false);

    expect(canDeleteTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-999",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sessionId: "999",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav",
    })).toBe(false);
  });

  it("only exposes playback for matching Yap-owned live audio", () => {
    const outputPath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt";
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav";

    expect(historyEntryPlaybackPath({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath,
      sessionId: "123",
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

  it("keeps a committed audio-only session actionable through its WAV identity", () => {
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-audio.wav";
    const entry = {
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-audio.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-audio",
      outputPath: sourcePath,
      sessionId: "audio",
      sourcePath,
    };

    expect(savedLiveSessionActionIdentity(entry)).toEqual({
      expectedCaptureCommitPath: entry.captureCommitPath,
      expectedOutputPath: sourcePath,
      sessionId: "audio",
    });
    expect(canDeleteTranscriptHistoryEntry(entry)).toBe(true);
    expect(historyEntryPlaybackPath(entry)).toBe(sourcePath);
  });

  it("uses a recovered partial row's source WAV as its native action identity", () => {
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.wav";
    const entry = {
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-recovered",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.txt",
      recoveryState: "recovered" as const,
      sessionId: "recovered",
      sourcePath,
    };

    expect(recoverableLiveSessionActionIdentity(entry)).toEqual({
      expectedArtifactPath: sourcePath,
      sessionId: "recovered",
    });
    expect(savedLiveSessionActionIdentity(entry)).toBeUndefined();
  });
});
