import { describe, expect, it, vi } from "vitest";

import {
  firstUnshownMaintenanceWarning,
  loadStableNativeLiveHistory,
  projectNativeLiveHistory,
  recordSavedLiveSession,
} from "@/hooks/use-live-history-sync";
import type { SavedLiveSession } from "@/live";

const dayMs = 24 * 60 * 60 * 1000;

function savedSession(overrides: Partial<SavedLiveSession> = {}): SavedLiveSession {
  return {
    captureCommitPath: "C:/Yap/live-100.commit.json",
    createdAtMs: Date.UTC(2026, 6, 11, 12),
    name: "live-100",
    outputPath: "C:/Yap/live-100.txt",
    sourcePath: "C:/Yap/live-100.wav",
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

describe("live history sync projections", () => {
  it("projects saved sessions before recoverable sessions with existing recovery fields", () => {
    const expiresAtMs = Date.UTC(2026, 6, 12, 15);

    const projection = projectNativeLiveHistory(
      {
        maintenanceWarnings: ["Primary warning", "Secondary warning"],
        sessions: [savedSession()],
      },
      [
        {
          audioPartialPath: null,
          expiresAtMs,
          journalPartialPath: "C:/Yap/live-200.partial.json",
          name: "live-200",
          reason: "Interrupted recording",
          sessionId: "200",
        },
      ],
    );

    expect(projection.entries.map((entry) => entry.name)).toEqual(["live-100", "live-200"]);
    expect(projection.entries[1]).toEqual({
      createdAt: new Date(expiresAtMs - dayMs).toISOString(),
      name: "live-200",
      outputPath: "C:/Yap/live-200.partial.json",
      recoveryState: "recoverable",
      sourcePath: "C:/Yap/live-200.partial.json",
      warning: "Interrupted recording",
    });
    expect(projection.maintenanceWarnings).toEqual(["Primary warning", "Secondary warning"]);
  });

  it("falls back to the recoverable name when no partial path exists", () => {
    const projection = projectNativeLiveHistory(
      { maintenanceWarnings: [], sessions: [] },
      [
        {
          expiresAtMs: dayMs,
          name: "live-orphan",
          reason: "No partial artifact",
          sessionId: "orphan",
        },
      ],
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

  it("retries a stale catalog when a saved event lands during reconciliation", async () => {
    const staleCatalog = deferred<{
      maintenanceWarnings: string[];
      sessions: SavedLiveSession[];
    }>();
    const staleRecoverable = deferred<[]>();
    const freshSaved = savedSession({ name: "live-new" });
    const listSavedSessions = vi.fn()
      .mockImplementationOnce(() => staleCatalog.promise)
      .mockResolvedValueOnce({ maintenanceWarnings: [], sessions: [freshSaved] });
    const listRecoverableSessions = vi.fn()
      .mockImplementationOnce(() => staleRecoverable.promise)
      .mockResolvedValueOnce([]);
    let savedGeneration = 0;

    const loading = loadStableNativeLiveHistory({
      getSavedGeneration: () => savedGeneration,
      isCancelled: () => false,
      listRecoverableSessions,
      listSavedSessions,
    });
    await vi.waitFor(() => expect(listSavedSessions).toHaveBeenCalledTimes(1));

    recordSavedLiveSession(savedSession(), () => true, () => {
      savedGeneration += 1;
    });
    staleCatalog.resolve({ maintenanceWarnings: [], sessions: [] });
    staleRecoverable.resolve([]);

    const projection = await loading;

    expect(listSavedSessions).toHaveBeenCalledTimes(2);
    expect(listRecoverableSessions).toHaveBeenCalledTimes(2);
    expect(projection?.entries.map((entry) => entry.name)).toEqual(["live-new"]);
  });
});
