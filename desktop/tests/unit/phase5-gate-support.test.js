import { describe, expect, it } from "vitest";

import {
  matchCompletedRemoteTranscript,
  matchesVerifiedHistoryDialog,
  resolvePhase5GateTimeout,
  sameWindowsPath,
} from "../wdio/phase5-gate-support.js";

describe("Phase 5 native gate support", () => {
  it("supplies the bounded default timeout when the operator does not override it", () => {
    expect(resolvePhase5GateTimeout(undefined)).toBe(2_700_000);
  });

  it("treats Windows case and extended-length prefixes as the same path", () => {
    expect(sameWindowsPath("C:\\Private\\Evidence", "\\\\?\\c:\\private\\evidence\\")).toBe(true);
  });

  it("joins the created remote job to its terminal History entry by session and source", () => {
    const createdJob = {
      id: "job-0123456789abcdef01234567",
      route: "serverBatch",
      sourcePath: "C:\\fixture.wav",
      status: "queued_server",
    };
    const historyEntry = {
      name: "fixture.wav",
      origin: "remote",
      outputPath: "C:\\Yap\\remote-jobs\\job-0123456789abcdef01234567\\result-1\\transcript.txt",
      sessionId: "s-0123456789abcdef01234567",
      sourcePath: "\\\\?\\c:\\fixture.wav",
    };
    const matched = matchCompletedRemoteTranscript(
      createdJob,
      {
        maintenanceWarnings: [],
        sessions: [historyEntry],
      },
    );

    expect(matched).toBe(historyEntry);
    expect(matchCompletedRemoteTranscript(
      { ...createdJob, route: "localFallback" },
      { maintenanceWarnings: [], sessions: [historyEntry] },
    )).toBeUndefined();
    expect(matchCompletedRemoteTranscript(
      { ...createdJob, id: "job-not-a-minted-id" },
      { maintenanceWarnings: [], sessions: [historyEntry] },
    )).toBeUndefined();
    expect(matchCompletedRemoteTranscript(
      createdJob,
      { maintenanceWarnings: [], sessions: [{ ...historyEntry, sessionId: "s-other" }] },
    )).toBeUndefined();
    expect(matchCompletedRemoteTranscript(
      createdJob,
      { maintenanceWarnings: [], sessions: [{ ...historyEntry, sourcePath: "C:\\other.wav" }] },
    )).toBeUndefined();
    expect(matchCompletedRemoteTranscript(
      createdJob,
      { maintenanceWarnings: [], sessions: [{ ...historyEntry, origin: "live" }] },
    )).toBeUndefined();
  });

  it("recognizes the exact verified transcript dialog without requiring the table behind it", () => {
    const name = "fixture.wav";
    const transcript = "Verified server transcript.";

    expect(matchesVerifiedHistoryDialog(
      [{ label: name, transcript }],
      name,
      transcript,
    )).toBe(true);
    expect(matchesVerifiedHistoryDialog(
      [{ label: "other.wav", transcript }],
      name,
      transcript,
    )).toBe(false);
    expect(matchesVerifiedHistoryDialog(
      [{ label: name, transcript: "Different text." }],
      name,
      transcript,
    )).toBe(false);
  });
});
