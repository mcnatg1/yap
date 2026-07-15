import { describe, expect, it, vi } from "vitest";

import {
  runDeleteRecoverableHistoryEntry,
  runDeleteSavedHistoryEntry,
  runHideHistoryEntry,
  runRecoverHistoryEntry,
  type HistoryActionPorts,
  type HistoryActionRuntime,
} from "@/hooks/use-history-actions";
import {
  savedSessionToTranscriptHistoryEntry,
} from "@/native-history";
import type { TranscriptHistoryEntry } from "@/history-model";
import type { SavedLiveSession } from "@/live";

function historyEntry(overrides: Partial<TranscriptHistoryEntry> = {}): TranscriptHistoryEntry {
  const entry: TranscriptHistoryEntry = {
    captureCommitPath: "C:/Yap/live-123.commit.json",
    createdAt: "2026-07-11T12:00:00.000Z",
    name: "live-123",
    outputPath: "C:/Yap/live-123.txt",
    sessionId: "123",
    sourcePath: "C:/Yap/live-123.wav",
    ...overrides,
  };
  if (!entry.sessionId) return entry;
  return savedSessionToTranscriptHistoryEntry({
    captureCommitPath: entry.captureCommitPath,
    createdAtMs: Date.parse(entry.createdAt),
    name: entry.name,
    outputPath: entry.outputPath,
    recoveryState: entry.recoveryState,
    sessionId: entry.sessionId,
    sourcePath: entry.sourcePath,
    warning: entry.warning,
  });
}

function savedSession(): SavedLiveSession {
  return {
    createdAtMs: Date.UTC(2026, 6, 11, 13),
    name: "live-123-recovered",
    outputPath: "C:/Yap/live-123-recovered.txt",
    sessionId: "123-recovered",
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
    }),
    forgetTranscriptText: vi.fn((path) => calls.push(`text:${path}`)),
    recordVisibleHistoryEntries: vi.fn((entries, warning) => {
      calls.push(`record:${entries[0]?.outputPath}:${warning}`);
      return true;
    }),
    rememberHiddenHistoryEntry: vi.fn(async (entry) => {
      calls.push(`hide:${entry.outputPath}`);
      return true;
    }),
    selectHistoryEntry: vi.fn((entry) => calls.push(`select:${entry.outputPath}`)),
    ...overrides,
  };
  const runtime: HistoryActionRuntime = {
    deleteRecoverableLiveSession: vi.fn(async (...args: string[]) => {
      calls.push(`delete-recoverable:${args.join("|")}`);
    }),
    deleteSavedLiveSession: vi.fn(async (...args: string[]) => {
      calls.push(`delete-saved:${args.join("|")}`);
    }),
    recoverLiveSession: vi.fn(async (...args: string[]) => {
      calls.push(`recover:${args.join("|")}`);
      return savedSession();
    }),
    showError: vi.fn((message) => calls.push(`error:${message}`)),
    showSuccess: vi.fn((message) => calls.push(`success:${message}`)),
  };
  return { calls, ports, runtime };
}

describe("history action ordering", () => {
  it("persists visibility before selection cleanup and success feedback", async () => {
    const { calls, ports, runtime } = actionHarness();

    await runHideHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual([
      "hide:C:/Yap/live-123.txt",
      "forget:C:/Yap/live-123.txt",
      "clear:C:/Yap/live-123.txt",
      "success:Hidden from history",
    ]);
  });

  it("stops hide cleanup when the visibility preference cannot be recorded", async () => {
    const { calls, ports, runtime } = actionHarness({
      rememberHiddenHistoryEntry: vi.fn(async () => {
        calls.push("hide:failed");
        return false;
      }),
    });

    await runHideHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual(["hide:failed"]);
  });

  it("finishes hide cleanup after visibility is durable when row persistence fails", async () => {
    const { calls, ports, runtime } = actionHarness({
      forgetHistoryEntry: vi.fn(() => {
        calls.push("forget:failed");
      }),
    });

    await runHideHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual([
      "hide:C:/Yap/live-123.txt",
      "forget:failed",
      "clear:C:/Yap/live-123.txt",
      "success:Hidden from history",
    ]);
  });

  it("deletes the native saved session before local cleanup", async () => {
    const { calls, ports, runtime } = actionHarness();

    await runDeleteSavedHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual([
      "delete-saved:123|C:/Yap/live-123.txt|C:/Yap/live-123.commit.json",
      "forget:C:/Yap/live-123.txt",
      "clear:C:/Yap/live-123.txt",
      "text:C:/Yap/live-123.txt",
      "success:Deleted from device",
    ]);
  });

  it("finishes device cleanup after native delete when local history persistence fails", async () => {
    const { calls, ports, runtime } = actionHarness({
      forgetHistoryEntry: vi.fn(() => {
        calls.push("forget:failed");
      }),
    });

    await runDeleteSavedHistoryEntry(historyEntry(), ports, runtime);

    expect(calls).toEqual([
      "delete-saved:123|C:/Yap/live-123.txt|C:/Yap/live-123.commit.json",
      "forget:failed",
      "clear:C:/Yap/live-123.txt",
      "text:C:/Yap/live-123.txt",
      "success:Deleted from device",
    ]);
  });

  it("uses the opaque session identity when the display name disagrees", async () => {
    const { calls, ports, runtime } = actionHarness();

    await runDeleteSavedHistoryEntry(historyEntry({ name: "live-999" }), ports, runtime);

    expect(calls[0]).toBe(
      "delete-saved:123|C:/Yap/live-123.txt|C:/Yap/live-123.commit.json",
    );
  });

  it("records recovery before old-row cleanup and continues when old cleanup fails", async () => {
    const { calls, ports, runtime } = actionHarness({
      forgetHistoryEntry: vi.fn((path) => {
        calls.push(`forget-failed:${path}`);
      }),
    });

    await runRecoverHistoryEntry(
      historyEntry({
        captureCommitPath: undefined,
        name: "live-partial-9",
        outputPath: "C:/Yap/live-partial-9.wav.part",
        recoveryState: "recoverable",
        sessionId: "partial-9",
        sourcePath: "C:/Yap/live-partial-9.wav.part",
      }),
      ports,
      runtime,
    );

    expect(calls).toEqual([
      "recover:partial-9|C:/Yap/live-partial-9.wav.part",
      "record:C:/Yap/live-123-recovered.txt:Transcript history could not be saved.",
      "forget-failed:C:/Yap/live-partial-9.wav.part",
      "clear:C:/Yap/live-partial-9.wav.part",
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

    await runRecoverHistoryEntry(historyEntry({
      captureCommitPath: undefined,
      outputPath: "C:/Yap/live-123.wav.part",
      recoveryState: "recoverable",
      sourcePath: "C:/Yap/live-123.wav.part",
    }), ports, runtime);

    expect(calls).toEqual(["recover:123|C:/Yap/live-123.wav.part", "record:failed"]);
  });

  it("deletes a recoverable session before row, selection, and text cleanup", async () => {
    const { calls, ports, runtime } = actionHarness();

    await runDeleteRecoverableHistoryEntry(historyEntry({
      captureCommitPath: undefined,
      outputPath: "C:/Yap/live-123.wav.part",
      recoveryState: "recoverable",
      sourcePath: "C:/Yap/live-123.wav.part",
    }), ports, runtime);

    expect(calls).toEqual([
      "delete-recoverable:123|C:/Yap/live-123.wav.part",
      "forget:C:/Yap/live-123.wav.part",
      "clear:C:/Yap/live-123.wav.part",
      "text:C:/Yap/live-123.wav.part",
      "success:Partial recording deleted",
    ]);
  });

  it("finishes recoverable cleanup after native delete when row persistence fails", async () => {
    const { calls, ports, runtime } = actionHarness({
      forgetHistoryEntry: vi.fn(() => {
        calls.push("forget:failed");
      }),
    });

    await runDeleteRecoverableHistoryEntry(historyEntry({
      captureCommitPath: undefined,
      outputPath: "C:/Yap/live-123.wav.part",
      recoveryState: "recoverable",
      sourcePath: "C:/Yap/live-123.wav.part",
    }), ports, runtime);

    expect(calls).toEqual([
      "delete-recoverable:123|C:/Yap/live-123.wav.part",
      "forget:failed",
      "clear:C:/Yap/live-123.wav.part",
      "text:C:/Yap/live-123.wav.part",
      "success:Partial recording deleted",
    ]);
  });

  it("uses the source WAV for both recovered-partial native actions", async () => {
    const { ports, runtime } = actionHarness();
    const sourcePath = "C:/Yap/live-123.wav";
    const recovered = historyEntry({
      outputPath: "C:/Yap/live-123.txt",
      recoveryState: "recovered",
      sourcePath,
    });

    await runRecoverHistoryEntry(recovered, ports, runtime);
    await runDeleteRecoverableHistoryEntry(recovered, ports, runtime);

    expect(runtime.recoverLiveSession).toHaveBeenCalledWith("123", sourcePath);
    expect(runtime.deleteRecoverableLiveSession).toHaveBeenCalledWith("123", sourcePath);
  });

  it("does not invoke native actions for a legacy row without an opaque identity", async () => {
    const { calls, ports, runtime } = actionHarness();
    const legacy = historyEntry({ sessionId: undefined });

    await runDeleteSavedHistoryEntry(legacy, ports, runtime);
    await runRecoverHistoryEntry(legacy, ports, runtime);
    await runDeleteRecoverableHistoryEntry(legacy, ports, runtime);

    expect(calls).toEqual([
      "error:Recording identity is no longer current. Refresh history and try again.",
      "error:Recording identity is no longer current. Refresh history and try again.",
      "error:Recording identity is no longer current. Refresh history and try again.",
    ]);
  });

  it("does not invoke native actions when the opaque identity disagrees with the artifacts", async () => {
    const { calls, ports, runtime } = actionHarness();
    const mismatched = historyEntry({ sessionId: "999" });

    await runDeleteSavedHistoryEntry(mismatched, ports, runtime);
    await runRecoverHistoryEntry({
      ...mismatched,
      captureCommitPath: undefined,
      recoveryState: "recoverable",
    }, ports, runtime);
    await runDeleteRecoverableHistoryEntry({
      ...mismatched,
      captureCommitPath: undefined,
      recoveryState: "recoverable",
    }, ports, runtime);

    expect(calls).toEqual([
      "error:Recording identity is no longer current. Refresh history and try again.",
      "error:Recording identity is no longer current. Refresh history and try again.",
      "error:Recording identity is no longer current. Refresh history and try again.",
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
    await runRecoverHistoryEntry(historyEntry({
      captureCommitPath: undefined,
      outputPath: "C:/Yap/live-123.wav.part",
      recoveryState: "recoverable",
      sourcePath: "C:/Yap/live-123.wav.part",
    }), ports, runtime);

    expect(calls).toEqual(["error:Delete failed", "error:Recovery failed"]);
  });
});
