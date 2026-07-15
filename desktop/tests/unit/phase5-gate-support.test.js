import { describe, expect, it } from "vitest";

import {
  matchCompletedRemoteTranscript,
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

  it("joins the public completed-job projection to its verified catalog entry by output", () => {
    const matched = matchCompletedRemoteTranscript(
      {
        id: "job-client-1",
        outputPath: "C:\\Yap\\remote-jobs\\job-client-1\\result-1\\transcript.txt",
        route: "serverBatch",
        status: "complete",
      },
      {
        maintenanceWarnings: [],
        sessions: [{
          name: "fixture.wav",
          outputPath: "\\\\?\\c:\\yap\\remote-jobs\\job-client-1\\result-1\\transcript.txt",
          sessionId: "s-client-1",
          sourcePath: "C:\\fixture.wav",
        }],
      },
    );

    expect(matched?.sessionId).toBe("s-client-1");
    expect(matchCompletedRemoteTranscript(
      { outputPath: matched?.outputPath, route: "localFallback", status: "complete" },
      { maintenanceWarnings: [], sessions: [matched] },
    )).toBeUndefined();
  });
});
