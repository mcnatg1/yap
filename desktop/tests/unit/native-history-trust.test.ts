import { describe, expect, it } from "vitest";

import { maxTranscriptHistoryEntries } from "@/history-model";
import { readVisibleTranscriptHistory } from "@/history-storage";
import {
  canDeleteTranscriptHistoryEntry,
  isNativeLiveTranscriptHistoryEntry,
  reconcileNativeTranscriptHistoryEntries,
  removeTranscriptHistory,
  savedLiveSessionActionIdentity,
  savedSessionToTranscriptHistoryEntry,
} from "@/native-history";

describe("native transcript history trust", () => {
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

  it("revokes native authority when a catalog refresh removes a session", () => {
    const entry = savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "C:/Yap/live-revoked.commit.json",
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-revoked",
      outputPath: "C:/Yap/live-revoked.txt",
      sessionId: "revoked",
      sourcePath: "C:/Yap/live-revoked.wav",
    });
    expect(isNativeLiveTranscriptHistoryEntry(entry)).toBe(true);

    reconcileNativeTranscriptHistoryEntries([entry], [], []);

    expect(isNativeLiveTranscriptHistoryEntry(entry)).toBe(false);
    expect(canDeleteTranscriptHistoryEntry(entry)).toBe(false);
  });

  it("revokes replaced and explicitly removed native identities", () => {
    const original = savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "C:/Yap/live-replaced.commit.json",
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-replaced",
      outputPath: "C:/Yap/live-replaced.txt",
      sessionId: "replaced",
      sourcePath: "C:/Yap/live-replaced.wav",
    });
    const replacement = savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "D:/Yap/live-replaced.commit.json",
      createdAtMs: Date.UTC(2026, 0, 2),
      name: "live-replaced",
      outputPath: "D:/Yap/live-replaced.txt",
      sessionId: "replaced",
      sourcePath: "D:/Yap/live-replaced.wav",
    });

    expect(isNativeLiveTranscriptHistoryEntry(original)).toBe(false);
    expect(isNativeLiveTranscriptHistoryEntry(replacement)).toBe(true);
    expect(removeTranscriptHistory([replacement], replacement.outputPath)).toEqual([]);
    expect(isNativeLiveTranscriptHistoryEntry(replacement)).toBe(false);
  });

  it("bounds native trust to the newest 500 catalog entries", () => {
    const entries = Array.from({ length: maxTranscriptHistoryEntries + 1 }, (_, index) => ({
      captureCommitPath: `C:/Yap/live-bounded-${index}.commit.json`,
      createdAt: new Date(Date.UTC(2026, 0, 1, 0, 0, index)).toISOString(),
      name: `live-bounded-${index}`,
      origin: "live" as const,
      outputPath: `C:/Yap/live-bounded-${index}.txt`,
      sessionId: `bounded-${index}`,
      sourcePath: `C:/Yap/live-bounded-${index}.wav`,
    }));

    reconcileNativeTranscriptHistoryEntries([], entries, []);

    expect(isNativeLiveTranscriptHistoryEntry(entries[0])).toBe(false);
    expect(isNativeLiveTranscriptHistoryEntry(entries[entries.length - 1])).toBe(true);
    reconcileNativeTranscriptHistoryEntries([], [], []);
  });

  it("does not accumulate native authority across more than 500 catalog refreshes", () => {
    const entries = Array.from({ length: maxTranscriptHistoryEntries + 1 }, (_, index) => ({
      captureCommitPath: `C:/Yap/live-refresh-${index}.commit.json`,
      createdAt: new Date(Date.UTC(2026, 0, 1, 0, 0, index)).toISOString(),
      name: `live-refresh-${index}`,
      origin: "live" as const,
      outputPath: `C:/Yap/live-refresh-${index}.txt`,
      sessionId: `refresh-${index}`,
      sourcePath: `C:/Yap/live-refresh-${index}.wav`,
    }));

    for (const entry of entries) {
      reconcileNativeTranscriptHistoryEntries([], [entry], []);
    }

    expect(isNativeLiveTranscriptHistoryEntry(entries[0])).toBe(false);
    expect(isNativeLiveTranscriptHistoryEntry(entries[entries.length - 1])).toBe(true);
    reconcileNativeTranscriptHistoryEntries([], [], []);
  });

  it("keeps forged persisted native rows hide-only after native rehydration", () => {
    const row = {
      captureCommitPath: "C:/Yap/live-forged.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-forged",
      outputPath: "C:/Yap/live-forged.txt",
      sessionId: "forged",
      sourcePath: "C:/Yap/live-forged.wav",
    };
    const storage = {
      getItem: () => JSON.stringify([row]),
      setItem: () => undefined,
    };
    const persisted = readVisibleTranscriptHistory(storage)[0];

    expect(canDeleteTranscriptHistoryEntry(persisted)).toBe(false);
    expect(savedLiveSessionActionIdentity(persisted)).toBeUndefined();

    const native = savedSessionToTranscriptHistoryEntry({
      captureCommitPath: row.captureCommitPath,
      createdAtMs: Date.parse(row.createdAt),
      name: row.name,
      origin: "live",
      outputPath: row.outputPath,
      sessionId: row.sessionId,
      sourcePath: row.sourcePath,
    });
    const rehydrated = readVisibleTranscriptHistory(storage)[0];
    expect(canDeleteTranscriptHistoryEntry(native)).toBe(true);
    expect(canDeleteTranscriptHistoryEntry(rehydrated)).toBe(false);
    expect(JSON.stringify(rehydrated)).not.toContain("trusted");
  });
});
