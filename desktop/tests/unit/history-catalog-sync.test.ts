import { describe, expect, it, vi } from "vitest";

import type { NativeHistoryCatalog } from "@/history-catalog";
import {
  acceptMaintenanceWarnings,
  historyCatalogEntryKey,
  prepareHistoryCatalogReconciliation,
  projectNativeHistoryCatalog,
  selectSavedHistoryCatalogEntry,
  subscribeHistoryCatalogEvents,
} from "@/hooks/use-history-catalog-sync";
import { createTranscriptHistoryStore } from "@/hooks/use-transcript-history";
import {
  readTranscriptHistory,
  readVisibleTranscriptHistory,
  savedSessionToTranscriptHistoryEntry,
  writeHiddenTranscriptHistory,
  writeTranscriptHistory,
  type HistoryStorage,
  type TranscriptHistoryEntry,
} from "@/history";

function memoryStorage(): HistoryStorage {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  };
}

function catalog(sessions: NativeHistoryCatalog["sessions"]): NativeHistoryCatalog {
  return { maintenanceWarnings: [], sessions };
}

function liveSession(
  sessionId: string,
  outputPath = `C:/Yap/live-${sessionId}.txt`,
): NativeHistoryCatalog["sessions"][number] {
  return {
    captureCommitPath: `C:/Yap/live-${sessionId}.commit.json`,
    createdAtMs: Date.UTC(2026, 6, 15, 12),
    name: `live-${sessionId}`,
    origin: "live",
    outputPath,
    sessionId,
    sourcePath: `C:/Yap/live-${sessionId}.wav`,
  };
}

function recoverableSession(sessionId: string): NativeHistoryCatalog["sessions"][number] {
  return {
    captureCommitPath: null,
    createdAtMs: Date.UTC(2026, 6, 15, 11),
    name: `live-${sessionId}`,
    origin: "live",
    outputPath: `C:/Yap/live-${sessionId}.wav.part`,
    recoveryState: "recoverable",
    sessionId,
    sourcePath: `C:/Yap/live-${sessionId}.wav.part`,
    warning: "Capture stopped before publication.",
  };
}

function createStore(storage: HistoryStorage) {
  let current = readVisibleTranscriptHistory(storage);
  const onWarning = vi.fn();
  const store = createTranscriptHistoryStore({
    getCurrentHistory: () => current,
    onWarning,
    replaceHistory: (next) => {
      current = next;
    },
    storage,
  });
  return { current: () => current, onWarning, store };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

describe("native history catalog synchronization", () => {
  it("projects explicit native provenance into history entries", () => {
    const entries = projectNativeHistoryCatalog(catalog([
      liveSession("live"),
      {
        createdAtMs: Date.UTC(2026, 6, 15, 11),
        name: "Remote",
        origin: "remote",
        outputPath: "C:/Yap/remote.txt",
        sessionId: "remote",
        sourcePath: "C:/Yap/source.wav",
      },
    ]));

    expect(entries.map((entry) => entry.origin)).toEqual(["live", "remote"]);
    expect(entries[0].createdAt).toBe(new Date(Date.UTC(2026, 6, 15, 12)).toISOString());
  });

  it("does not announce the initial catalog and selects only a later or preferred save", () => {
    const initial = projectNativeHistoryCatalog(catalog([liveSession("one")]));
    const known = new Set(initial.map(historyCatalogEntryKey));
    expect(selectSavedHistoryCatalogEntry(initial, new Set(), false)).toBeUndefined();

    const remote = savedSessionToTranscriptHistoryEntry({
      createdAtMs: 2,
      name: "Remote",
      origin: "remote",
      outputPath: "remote.txt",
      sessionId: "remote",
      sourcePath: "source.wav",
    });
    const next = [remote, ...initial];
    expect(selectSavedHistoryCatalogEntry(next, known, true)).toBe(remote);
    expect(
      selectSavedHistoryCatalogEntry(next, known, false, historyCatalogEntryKey(initial[0])),
    ).toBe(initial[0]);
  });

  it("shows at most one warning from a catalog while remembering the entire batch", () => {
    const shown = new Set<string>();
    expect(acceptMaintenanceWarnings(["First", "Second"], shown)).toBe("First");
    expect(acceptMaintenanceWarnings(["First", "Second"], shown)).toBeUndefined();
    expect(acceptMaintenanceWarnings(["Second", "Third"], shown)).toBe("Third");
  });

  it("retains a healthy event subscription when its sibling cannot be installed", async () => {
    const disposeLive = vi.fn();
    const liveSaved = vi.fn();
    const recordingJobsChanged = vi.fn();
    const subscriptions = await subscribeHistoryCatalogEvents(
      liveSaved,
      recordingJobsChanged,
      {
        listenLiveSaved: vi.fn(async (handler) => {
          expect(handler).toBe(liveSaved);
          return disposeLive;
        }),
        listenRecordingJobsChanged: vi.fn(async () => {
          throw new Error("listener unavailable");
        }),
      },
    );

    expect(subscriptions.failures).toEqual([expect.any(Error)]);
    subscriptions.dispose();
    subscriptions.dispose();
    expect(disposeLive).toHaveBeenCalledTimes(1);
  });

  it("preserves a concurrently accepted live save across an older in-flight catalog", async () => {
    const storage = memoryStorage();
    const harness = createStore(storage);
    const pendingCatalog = deferred<NativeHistoryCatalog>();
    const reconciliation = prepareHistoryCatalogReconciliation(
      harness.store.captureNativeHistoryReconciliation,
      () => pendingCatalog.promise,
    );
    const accepted = savedSessionToTranscriptHistoryEntry(liveSession("same", "C:/Yap/new.txt"));
    expect(harness.store.recordVisibleHistoryEntries([accepted], "save failed")).toBe(true);

    pendingCatalog.resolve(catalog([liveSession("same", "C:/Yap/stale.txt")]));
    const prepared = await reconciliation;
    expect(prepared.apply(prepared.entries, "sync failed")).toEqual([
      expect.objectContaining({ outputPath: "C:/Yap/new.txt" }),
    ]);

    expect(harness.current().map((entry) => entry.outputPath)).toEqual(["C:/Yap/new.txt"]);
    expect(readTranscriptHistory(storage)).toEqual([]);
  });

  it("lets a concurrent recovered save supersede stale recoverable metadata", async () => {
    const storage = memoryStorage();
    const harness = createStore(storage);
    const pendingCatalog = deferred<NativeHistoryCatalog>();
    const reconciliation = prepareHistoryCatalogReconciliation(
      harness.store.captureNativeHistoryReconciliation,
      () => pendingCatalog.promise,
    );
    const accepted = savedSessionToTranscriptHistoryEntry(liveSession("shared"));
    expect(harness.store.recordVisibleHistoryEntries([accepted], "save failed")).toBe(true);

    pendingCatalog.resolve(catalog([recoverableSession("shared")]));
    const prepared = await reconciliation;
    const visible = prepared.apply(prepared.entries, "sync failed");

    expect(visible).toEqual([
      expect.objectContaining({
        captureCommitPath: "C:/Yap/live-shared.commit.json",
        outputPath: "C:/Yap/live-shared.txt",
      }),
    ]);
    expect(visible?.[0].recoveryState).toBeUndefined();
  });

  it("migrates native browser projections while preserving unrelated legacy rows", async () => {
    const storage = memoryStorage();
    const legacy: TranscriptHistoryEntry = {
      createdAt: "2026-07-10T00:00:00.000Z",
      name: "Legacy import",
      outputPath: "C:/Legacy/import.txt",
      sourcePath: "C:/Legacy/import.wav",
    };
    const oldNativeProjection: TranscriptHistoryEntry = {
      captureCommitPath: "C:/Yap/live-old.commit.json",
      createdAt: "2026-07-11T00:00:00.000Z",
      name: "live-old",
      outputPath: "C:/Yap/live-old.txt",
      sessionId: "old",
      sourcePath: "C:/Yap/live-old.wav",
    };
    writeTranscriptHistory([oldNativeProjection, legacy], storage);
    const harness = createStore(storage);
    const prepared = await prepareHistoryCatalogReconciliation(
      harness.store.captureNativeHistoryReconciliation,
      async () => catalog([liveSession("old")]),
    );

    expect(prepared.apply(prepared.entries, "sync failed")).toEqual([
      expect.objectContaining({ name: "live-old" }),
      legacy,
    ]);
    expect(harness.current().map((entry) => entry.name)).toEqual(["live-old", "Legacy import"]);
    expect(readTranscriptHistory(storage)).toEqual([legacy]);
  });

  it("does not expose a hidden native save as a notification candidate", async () => {
    const storage = memoryStorage();
    const hidden = liveSession("hidden");
    writeHiddenTranscriptHistory([hidden.outputPath], storage);
    const harness = createStore(storage);
    const prepared = await prepareHistoryCatalogReconciliation(
      harness.store.captureNativeHistoryReconciliation,
      async () => catalog([hidden]),
    );

    const visibleEntries = prepared.apply(prepared.entries, "sync failed");
    expect(visibleEntries).toEqual([]);
    expect(selectSavedHistoryCatalogEntry(
      visibleEntries ?? [],
      new Set(),
      true,
      historyCatalogEntryKey(prepared.entries[0]),
    )).toBeUndefined();
    expect(harness.current()).toEqual([]);
  });

  it("applies each captured catalog reconciliation only once", async () => {
    const storage = memoryStorage();
    const harness = createStore(storage);
    const prepared = await prepareHistoryCatalogReconciliation(
      harness.store.captureNativeHistoryReconciliation,
      async () => catalog([liveSession("once")]),
    );

    expect(prepared.apply(prepared.entries, "sync failed")).toHaveLength(1);
    expect(prepared.apply([], "sync failed")).toBeUndefined();
    expect(harness.current()).toHaveLength(1);
  });

  it("does not publish a native projection when legacy migration persistence fails", async () => {
    const storage: HistoryStorage = {
      getItem: () => null,
      setItem: () => {
        throw new Error("storage unavailable");
      },
    };
    const harness = createStore(storage);
    const prepared = await prepareHistoryCatalogReconciliation(
      harness.store.captureNativeHistoryReconciliation,
      async () => catalog([liveSession("blocked")]),
    );

    expect(prepared.apply(prepared.entries, "sync failed")).toBeUndefined();
    expect(harness.current()).toEqual([]);
    expect(harness.onWarning).toHaveBeenCalledWith("sync failed", expect.any(Error));
  });
});
