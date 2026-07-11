import { describe, expect, it, vi } from "vitest";

import {
  runDeleteRecoverableHistoryEntry,
  runDeleteSavedHistoryEntry,
  runHideHistoryEntry,
  runRecoverHistoryEntry,
  type HistoryActionPorts,
  type HistoryActionRuntime,
} from "@/hooks/use-history-actions";
import type { TranscriptHistoryEntry } from "@/history";
import type { SavedLiveSession } from "@/live";

function historyEntry(overrides: Partial<TranscriptHistoryEntry> = {}): TranscriptHistoryEntry {
  return {
    createdAt: "2026-07-11T12:00:00.000Z",
    name: "live-123",
    outputPath: "C:/Yap/live-123.txt",
    sourcePath: "C:/Yap/live-123.wav",
    ...overrides,
  };
}

function savedSession(): SavedLiveSession {
  return {
    createdAtMs: Date.UTC(2026, 6, 11, 13),
    name: "live-123-recovered",
    outputPath: "C:/Yap/live-123-recovered.txt",
    sourcePath: "C:/Yap/live-123-recovered.wav",
    recoveryState: "recovered",
  };
}

function actionHarness(overrides: Partial<HistoryActionPorts> = {}) {
  const calls: string[] = [];
  const ports: HistoryActionPorts = {
    clearHistorySelectionIf: vi.fn((path) => calls.push(`clear:${path}`)),
    forgetHistoryEntry: vi.fn((path) => {
      calls.push(`forget:${path}`);
      return true;
    }),
    forgetTranscriptText: vi.fn((path) => calls.push(`text:${path}`)),
    recordVisibleHistoryEntries: vi.fn((entries, warning) => {
      calls.push(`record:${entries[0]?.outputPath}:${warning}`);
      return true;
    }),
    rememberHiddenHistoryEntry: vi.fn((path) => {
      calls.push(`hide:${path}`);
      return true;
    }),
    selectHistoryEntry: vi.fn((entry) => calls.push(`select:${entry.outputPath}`)),
    ...overrides,
  };
  const runtime: HistoryActionRuntime = {
    deleteRecoverableLiveSession: vi.fn(async (id) => {
      calls.push(`delete-recoverable:${id}`);
    }),
    deleteSavedLiveSession: vi.fn(async (id) => {
      calls.push(`delete-saved:${id}`);
    }),
    recoverLiveSession: vi.fn(async (id) => {
      calls.push(`recover:${id}`);
      return savedSession();
    }),
    showError: vi.fn((message) => calls.push(`error:${message}`)),
    showSuccess: vi.fn((message) => calls.push(`success:${message}`)),
  };
  return { calls, ports, runtime };
}

describe("history action ordering", () => {
  it("hides locally before selection cleanup and success feedback", () => {
    const { calls, ports, runtime } = actionHarness();

    runHideHistoryEntry("C:/Yap/live-123.txt", ports, runtime);

    expect(calls).toEqual([
      "hide:C:/Yap/live-123.txt",
      "forget:C:/Yap/live-123.txt",
      "clear:C:/Yap/live-123.txt",
      "success:Hidden from history",
    ]);
  });

  it("stops hide cleanup when the tombstone cannot be recorded", () => {
    const { calls, ports, runtime } = actionHarness({
      rememberHiddenHistoryEntry: vi.fn(() => {
        calls.push("hide:failed");
        return false;
      }),
    });

    runHideHistoryEntry("C:/Yap/live-123.txt", ports, runtime);

    expect(calls).toEqual(["hide:failed"]);
  });

  it("deletes the native saved session before local cleanup", async () => {
    const { calls, ports, runtime } = actionHarness();

    await runDeleteSavedHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual([
      "delete-saved:123",
      "hide:C:/Yap/live-123.txt",
      "forget:C:/Yap/live-123.txt",
      "clear:C:/Yap/live-123.txt",
      "text:C:/Yap/live-123.txt",
      "success:Deleted from device",
    ]);
  });

  it("preserves the partial failure after native delete succeeds", async () => {
    const { calls, ports, runtime } = actionHarness({
      rememberHiddenHistoryEntry: vi.fn(() => {
        calls.push("hide:failed");
        return false;
      }),
    });

    await runDeleteSavedHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual(["delete-saved:123", "hide:failed"]);
  });

  it("records recovery before old-row cleanup and continues when old cleanup fails", async () => {
    const { calls, ports, runtime } = actionHarness({
      forgetHistoryEntry: vi.fn((path) => {
        calls.push(`forget-failed:${path}`);
        return false;
      }),
    });

    await runRecoverHistoryEntry(
      historyEntry({
        name: "live-partial-9",
        outputPath: "C:/Yap/live-partial-9.partial.json",
        recoveryState: "recoverable",
      }),
      ports,
      runtime,
    );

    expect(calls).toEqual([
      "recover:partial-9",
      "record:C:/Yap/live-123-recovered.txt:Transcript history could not be saved.",
      "forget-failed:C:/Yap/live-partial-9.partial.json",
      "clear:C:/Yap/live-partial-9.partial.json",
      "select:C:/Yap/live-123-recovered.txt",
      "success:Partial recording recovered",
    ]);
  });

  it("stops recovery cleanup when the recovered row cannot be recorded", async () => {
    const { calls, ports, runtime } = actionHarness({
      recordVisibleHistoryEntries: vi.fn(() => {
        calls.push("record:failed");
        return false;
      }),
    });

    await runRecoverHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual(["recover:123", "record:failed"]);
  });

  it("deletes a recoverable session before row, selection, and text cleanup", async () => {
    const { calls, ports, runtime } = actionHarness();

    await runDeleteRecoverableHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual([
      "delete-recoverable:123",
      "forget:C:/Yap/live-123.txt",
      "clear:C:/Yap/live-123.txt",
      "text:C:/Yap/live-123.txt",
      "success:Partial recording deleted",
    ]);
  });

  it("keeps exact fallback error strings for failed native actions", async () => {
    const { calls, ports, runtime } = actionHarness();
    runtime.deleteSavedLiveSession = vi.fn(async () => {
      throw "";
    });
    runtime.recoverLiveSession = vi.fn(async () => {
      throw "";
    });

    await runDeleteSavedHistoryEntry(historyEntry(), ports, runtime);
    await runRecoverHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual(["error:Delete failed", "error:Recovery failed"]);
  });
});
