import { describe, expect, it, vi } from "vitest";

import {
  firstUnshownMaintenanceWarning,
  projectNativeLiveHistory,
  recordSavedLiveSession,
  syncNativeLiveHistory,
} from "@/hooks/use-live-history-sync";
import { createTranscriptHistoryStore } from "@/hooks/use-transcript-history";
import {
  readVisibleTranscriptHistory,
  writeHiddenTranscriptHistory,
  type HistoryStorage,
  type TranscriptHistoryEntry,
} from "@/history";
import type {
  RecoverableLiveSession,
  SavedLiveSession,
  SavedLiveSessionCatalog,
} from "@/live";

const dayMs = 24 * 60 * 60 * 1000;

function savedSession(overrides: Partial<SavedLiveSession> = {}): SavedLiveSession {
  const name = overrides.name ?? "live-100";
  return {
    captureCommitPath: `C:/Yap/${name}.commit.json`,
    createdAtMs: Date.UTC(2026, 6, 11, 12),
    name,
    outputPath: `C:/Yap/${name}.txt`,
    sourcePath: `C:/Yap/${name}.wav`,
    ...overrides,
  };
}

function recoverableSession(
  overrides: Partial<RecoverableLiveSession> = {},
): RecoverableLiveSession {
  const sessionId = overrides.sessionId ?? "200";
  const name = overrides.name ?? `live-${sessionId}`;
  return {
    audioPartialPath: `C:/Yap/${name}.wav.part`,
    expiresAtMs: Date.UTC(2026, 6, 12, 15),
    journalPartialPath: `C:/Yap/${name}.capture.partial.json`,
    name,
    reason: "Interrupted recording",
    sessionId,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

function memoryStorage(): HistoryStorage {
  const values = new Map<string, string>();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

function createHistoryStoreHarness(hiddenOutputPaths: string[] = []) {
  const storage = memoryStorage();
  if (hiddenOutputPaths.length) {
    writeHiddenTranscriptHistory(hiddenOutputPaths, storage);
  }
  let currentHistory = readVisibleTranscriptHistory(storage);
  const replacements: TranscriptHistoryEntry[][] = [];
  const onWarning = vi.fn();
  const store = createTranscriptHistoryStore({
    getCurrentHistory: () => currentHistory,
    onWarning,
    replaceHistory: (next) => {
      currentHistory = next;
      replacements.push(next);
    },
    storage,
  });

  return {
    currentHistory: () => currentHistory,
    onWarning,
    persistedHistory: () => readVisibleTranscriptHistory(storage),
    replacements,
    store,
  };
}

function guardedMaintenanceWarning(onWarning: (warning: string) => void) {
  let shown = false;
  return (warnings: string[]) => {
    const warning = firstUnshownMaintenanceWarning(warnings, shown);
    if (!warning) return;
    shown = true;
    onWarning(warning);
  };
}

function startDeferredSync(
  captureNativeHistoryReconciliation: ReturnType<typeof createHistoryStoreHarness>["store"]["captureNativeHistoryReconciliation"],
  options: { isCancelled?: () => boolean; onMaintenanceWarning?: (warning: string) => void } = {},
) {
  const catalog = deferred<SavedLiveSessionCatalog>();
  const recoverable = deferred<RecoverableLiveSession[]>();
  const listSavedSessions = vi.fn(() => catalog.promise);
  const listRecoverableSessions = vi.fn(() => recoverable.promise);
  const onMaintenanceWarning = options.onMaintenanceWarning ?? vi.fn();
  const synchronizing = syncNativeLiveHistory({
    captureNativeHistoryReconciliation,
    isCancelled: options.isCancelled ?? (() => false),
    listRecoverableSessions,
    listSavedSessions,
    onMaintenanceWarnings: guardedMaintenanceWarning(onMaintenanceWarning),
  });

  return {
    catalog,
    listRecoverableSessions,
    listSavedSessions,
    onMaintenanceWarning,
    recoverable,
    synchronizing,
  };
}

describe("live history sync projections", () => {
  it("projects saved sessions before recoverable sessions with existing recovery fields", () => {
    const expiresAtMs = Date.UTC(2026, 6, 12, 15);

    const projection = projectNativeLiveHistory(
      {
        maintenanceWarnings: ["Primary warning", "Secondary warning"],
        sessions: [savedSession()],
      },
      [recoverableSession({ expiresAtMs })],
    );

    expect(projection.entries.map((entry) => entry.name)).toEqual(["live-100", "live-200"]);
    expect(projection.entries[1]).toEqual({
      createdAt: new Date(expiresAtMs - dayMs).toISOString(),
      name: "live-200",
      outputPath: "C:/Yap/live-200.wav.part",
      recoveryState: "recoverable",
      sourcePath: "C:/Yap/live-200.wav.part",
      warning: "Interrupted recording",
    });
    expect(projection.maintenanceWarnings).toEqual(["Primary warning", "Secondary warning"]);
  });

  it("falls back to the recoverable name when no partial path exists", () => {
    const projection = projectNativeLiveHistory(
      { maintenanceWarnings: [], sessions: [] },
      [recoverableSession({
        audioPartialPath: null,
        expiresAtMs: dayMs,
        journalPartialPath: null,
        name: "live-orphan",
        sessionId: "orphan",
      })],
    );

    expect(projection.entries[0]).toMatchObject({
      outputPath: "live-orphan",
      sourcePath: "live-orphan",
    });
  });

  it("selects only the first maintenance warning before the one-shot guard is set", () => {
    expect(firstUnshownMaintenanceWarning(["First", "Second"], false)).toBe("First");
    expect(firstUnshownMaintenanceWarning(["First", "Second"], true)).toBeUndefined();
    expect(firstUnshownMaintenanceWarning([], false)).toBeUndefined();
  });

  it("does not run saved-session reactions when the history store rejects the row", () => {
    const recordVisibleHistoryEntries = vi.fn(() => false);
    const onSaved = vi.fn();

    expect(
      recordSavedLiveSession(savedSession(), recordVisibleHistoryEntries, onSaved),
    ).toBe(false);
    expect(recordVisibleHistoryEntries).toHaveBeenCalledWith(
      [expect.objectContaining({ name: "live-100" })],
      "Transcript history could not be saved.",
    );
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("runs saved-session reactions only after the history store accepts the row", () => {
    const recordVisibleHistoryEntries = vi.fn(() => true);
    const onSaved = vi.fn();

    expect(
      recordSavedLiveSession(savedSession(), recordVisibleHistoryEntries, onSaved),
    ).toBe(true);
    expect(onSaved).toHaveBeenCalledWith(expect.objectContaining({ name: "live-100" }));
    expect(recordVisibleHistoryEntries.mock.invocationCallOrder[0]).toBeLessThan(
      onSaved.mock.invocationCallOrder[0],
    );
  });
});

describe("live history synchronization", () => {
  it("preserves accepted canonical rows after the baseline when the one native pair is stale", async () => {
    const history = createHistoryStoreHarness();
    expect(recordSavedLiveSession(
      savedSession({ name: "live-before-baseline" }),
      history.store.recordVisibleHistoryEntries,
      vi.fn(),
    )).toBe(true);
    const sync = startDeferredSync(history.store.captureNativeHistoryReconciliation);
    await vi.waitFor(() => expect(sync.listSavedSessions).toHaveBeenCalledTimes(1));

    expect(recordSavedLiveSession(
      savedSession({ name: "live-concurrent" }),
      history.store.recordVisibleHistoryEntries,
      vi.fn(),
    )).toBe(true);
    sync.catalog.resolve({ maintenanceWarnings: [], sessions: [] });
    sync.recoverable.resolve([]);
    await sync.synchronizing;

    expect(history.persistedHistory().map((entry) => entry.name)).toEqual([
      "live-concurrent",
    ]);
    expect(sync.listSavedSessions).toHaveBeenCalledTimes(1);
    expect(sync.listRecoverableSessions).toHaveBeenCalledTimes(1);
  });

  it("completes under sustained accepted-save churn with one finite native pair", async () => {
    const history = createHistoryStoreHarness();
    const sync = startDeferredSync(history.store.captureNativeHistoryReconciliation);
    await vi.waitFor(() => expect(sync.listSavedSessions).toHaveBeenCalledTimes(1));
    const expectedNames = Array.from({ length: 64 }, (_, index) => `live-churn-${index}`);

    for (const [index, name] of expectedNames.entries()) {
      expect(recordSavedLiveSession(
        savedSession({ createdAtMs: Date.UTC(2026, 6, 11, 12, 0, index), name }),
        history.store.recordVisibleHistoryEntries,
        vi.fn(),
      )).toBe(true);
    }
    sync.catalog.resolve({ maintenanceWarnings: [], sessions: [] });
    sync.recoverable.resolve([]);
    await sync.synchronizing;

    expect(new Set(history.persistedHistory().map((entry) => entry.name))).toEqual(
      new Set(expectedNames),
    );
    expect(sync.listSavedSessions).toHaveBeenCalledTimes(1);
    expect(sync.listRecoverableSessions).toHaveBeenCalledTimes(1);
  });

  it("does not reconcile or warn after cancellation while the bounded pair resolves", async () => {
    const history = createHistoryStoreHarness();
    let cancelled = false;
    let completed = false;
    const sync = startDeferredSync(history.store.captureNativeHistoryReconciliation, {
      isCancelled: () => cancelled,
    });
    const completion = sync.synchronizing.then(() => {
      completed = true;
    });
    await vi.waitFor(() => expect(sync.listSavedSessions).toHaveBeenCalledTimes(1));

    cancelled = true;
    sync.catalog.resolve({ maintenanceWarnings: ["Maintenance needed"], sessions: [savedSession()] });
    sync.recoverable.resolve([recoverableSession()]);
    await completion;

    expect(completed).toBe(true);
    expect(history.replacements).toHaveLength(0);
    expect(sync.onMaintenanceWarning).not.toHaveBeenCalled();
    expect(sync.listSavedSessions).toHaveBeenCalledTimes(1);
    expect(sync.listRecoverableSessions).toHaveBeenCalledTimes(1);
  });

  it("carries the in-flight maintenance warning through concurrent accepted saves", async () => {
    const history = createHistoryStoreHarness();
    const onMaintenanceWarning = vi.fn();
    const sync = startDeferredSync(history.store.captureNativeHistoryReconciliation, {
      onMaintenanceWarning,
    });
    await vi.waitFor(() => expect(sync.listSavedSessions).toHaveBeenCalledTimes(1));

    expect(recordSavedLiveSession(
      savedSession({ name: "live-during-warning" }),
      history.store.recordVisibleHistoryEntries,
      vi.fn(),
    )).toBe(true);
    sync.catalog.resolve({
      maintenanceWarnings: ["Maintenance needed", "Secondary warning"],
      sessions: [],
    });
    sync.recoverable.resolve([]);
    await sync.synchronizing;

    expect(onMaintenanceWarning).toHaveBeenCalledTimes(1);
    expect(onMaintenanceWarning).toHaveBeenCalledWith("Maintenance needed");
    expect(history.persistedHistory().map((entry) => entry.name)).toEqual([
      "live-during-warning",
    ]);
  });

  it("does not preserve or react to a hidden saved event", async () => {
    const hiddenSession = savedSession({ name: "live-hidden" });
    const history = createHistoryStoreHarness([hiddenSession.outputPath]);
    const onSaved = vi.fn();
    const sync = startDeferredSync(history.store.captureNativeHistoryReconciliation);
    await vi.waitFor(() => expect(sync.listSavedSessions).toHaveBeenCalledTimes(1));

    expect(recordSavedLiveSession(
      hiddenSession,
      history.store.recordVisibleHistoryEntries,
      onSaved,
    )).toBe(false);
    sync.catalog.resolve({ maintenanceWarnings: [], sessions: [hiddenSession] });
    sync.recoverable.resolve([]);
    await sync.synchronizing;

    expect(history.persistedHistory()).toEqual([]);
    expect(onSaved).not.toHaveBeenCalled();
    expect(sync.listSavedSessions).toHaveBeenCalledTimes(1);
    expect(sync.listRecoverableSessions).toHaveBeenCalledTimes(1);
  });

  it("lets a concurrent accepted canonical session supersede stale recovery metadata", async () => {
    const history = createHistoryStoreHarness();
    const sync = startDeferredSync(history.store.captureNativeHistoryReconciliation);
    await vi.waitFor(() => expect(sync.listSavedSessions).toHaveBeenCalledTimes(1));
    const canonical = savedSession({ name: "live-shared" });

    expect(recordSavedLiveSession(
      canonical,
      history.store.recordVisibleHistoryEntries,
      vi.fn(),
    )).toBe(true);
    sync.catalog.resolve({ maintenanceWarnings: [], sessions: [] });
    sync.recoverable.resolve([recoverableSession({ name: canonical.name, sessionId: "shared" })]);
    await sync.synchronizing;

    expect(history.persistedHistory()).toEqual([expect.objectContaining({
      captureCommitPath: canonical.captureCommitPath,
      name: canonical.name,
      outputPath: canonical.outputPath,
    })]);
    expect(history.persistedHistory()[0]).not.toHaveProperty("recoveryState");
    expect(history.persistedHistory()[0]).not.toHaveProperty("warning");
  });

  it("applies a captured native reconciliation closure only once", () => {
    const history = createHistoryStoreHarness();
    const apply = history.store.captureNativeHistoryReconciliation();
    expect(recordSavedLiveSession(
      savedSession({ name: "live-one-use" }),
      history.store.recordVisibleHistoryEntries,
      vi.fn(),
    )).toBe(true);

    expect(apply([], "sync warning")).toBe(true);
    expect(apply([], "sync warning")).toBe(false);
    expect(history.persistedHistory().map((entry) => entry.name)).toEqual(["live-one-use"]);
  });
});
