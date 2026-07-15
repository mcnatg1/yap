import { describe, expect, it, vi } from "vitest";

import {
  projectCompletedRemoteHistory,
  recordCompletedRemoteHistory,
  remoteHistoryResultKey,
} from "@/hooks/use-remote-history-sync";

const catalog = {
  maintenanceWarnings: [],
  sessions: [{
    createdAtMs: Date.UTC(2026, 6, 14, 21),
    name: "meeting.wav",
    outputPath: "C:/Yap/remote-jobs/job-1/result-00000000000000000001/transcript.txt",
    sessionId: "s-job-1",
    sourcePath: "C:/Recordings/meeting.wav",
  }],
};

describe("private-server transcript history sync", () => {
  it("projects a completed native catalog row without claiming live-capture ownership", () => {
    expect(projectCompletedRemoteHistory(catalog)).toEqual([{
      createdAt: "2026-07-14T21:00:00.000Z",
      name: "meeting.wav",
      outputPath: catalog.sessions[0].outputPath,
      sessionId: "s-job-1",
      sourcePath: "C:/Recordings/meeting.wav",
    }]);
  });

  it("notifies only after the history store accepts the verified row", () => {
    const reject = vi.fn(() => false);
    const accept = vi.fn(() => true);
    const onSaved = vi.fn();

    expect(recordCompletedRemoteHistory(catalog, reject, onSaved)).toBe(false);
    expect(onSaved).not.toHaveBeenCalled();
    expect(recordCompletedRemoteHistory(catalog, accept, onSaved)).toBe(true);
    expect(accept).toHaveBeenCalledWith(
      [expect.objectContaining({ sessionId: "s-job-1" })],
      "Private-server transcript history could not be saved.",
    );
    expect(onSaved).toHaveBeenCalledWith(expect.objectContaining({ name: "meeting.wav" }));
  });

  it("does not re-notify an immutable result on unrelated queue refreshes", () => {
    const accept = vi.fn(() => true);
    const onSaved = vi.fn();
    const entry = projectCompletedRemoteHistory(catalog)[0];
    const alreadyRecorded = new Set([remoteHistoryResultKey(entry)]);

    expect(
      recordCompletedRemoteHistory(catalog, accept, onSaved, alreadyRecorded),
    ).toBe(true);
    expect(accept).toHaveBeenCalledOnce();
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("treats an empty first catalog as an initialized baseline", () => {
    const record = vi.fn(() => false);
    const onSaved = vi.fn();

    expect(recordCompletedRemoteHistory(
      { maintenanceWarnings: [], sessions: [] },
      record,
      onSaved,
    )).toBe(true);
    expect(record).not.toHaveBeenCalled();
    expect(onSaved).not.toHaveBeenCalled();
  });
});
