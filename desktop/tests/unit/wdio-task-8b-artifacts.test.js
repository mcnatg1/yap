import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

import { assertOwnedSavedSession } from "../wdio/task-8b-helpers.js";
import { installTask8bPrivateIsolationFixture } from "./wdio-task-8b-fixture.js";

const {
  privateIsolation,
  writeCanonicalBundle,
} = installTask8bPrivateIsolationFixture();


describe("Task 8b canonical saved-session ownership", () => {
  function fixture() {
    const isolation = privateIsolation("owned-recordings");
    return {
      isolation,
      recordingRoot: isolation.recordingRoot,
      saved: writeCanonicalBundle(isolation.recordingRoot),
    };
  }

  it("accepts one exact current-run bundle under the canonical isolated root", () => {
    const { recordingRoot, saved } = fixture();
    const owned = assertOwnedSavedSession(saved, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    });

    expect(owned.sessionId).toBe("s-18c13f2a28c8be80-d018-2");
    expect(owned.artifactNames).toEqual([
      "live-s-18c13f2a28c8be80-d018-2.capture.json",
      "live-s-18c13f2a28c8be80-d018-2.commit.json",
      "live-s-18c13f2a28c8be80-d018-2.transcript.r1.json",
      "live-s-18c13f2a28c8be80-d018-2.txt",
      "live-s-18c13f2a28c8be80-d018-2.wav",
    ]);
  });

  it("rejects relative paths and a different parent", () => {
    const { isolation, recordingRoot, saved } = fixture();
    expect(() => assertOwnedSavedSession({ ...saved, sourcePath: `${saved.name}.wav` }, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/absolute Windows path/i);

    const foreignRoot = path.join(isolation.runRoot, "foreign-recordings");
    mkdirSync(foreignRoot);
    const foreignWav = path.join(foreignRoot, `${saved.name}.wav`);
    writeFileSync(foreignWav, Buffer.alloc(64));
    expect(() => assertOwnedSavedSession({ ...saved, sourcePath: foreignWav }, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/isolated recording root/i);
  });

  it("rejects a canonical path escape even when the lexical parent matches", () => {
    const { isolation, recordingRoot, saved } = fixture();
    const foreignRoot = path.join(isolation.runRoot, "canonical-escape");
    mkdirSync(foreignRoot);
    const foreignWav = path.join(foreignRoot, `${saved.name}.wav`);
    writeFileSync(foreignWav, Buffer.alloc(64));
    const canonicalize = (candidate) => candidate === saved.sourcePath ? foreignWav : candidate;

    expect(() => assertOwnedSavedSession(saved, recordingRoot, {
      canonicalize,
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/canonical parent/i);
  });

  it("rejects a stale event, name/path mismatch, missing suffix, or foreign root entry", () => {
    const { recordingRoot, saved } = fixture();
    expect(() => assertOwnedSavedSession({ ...saved, createdAtMs: 1_000 }, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/current test run/i);
    expect(() => assertOwnedSavedSession({ ...saved, name: "live-s-1-2-3" }, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/opaque session ID/i);

    writeFileSync(path.join(recordingRoot, `${saved.name}.unexpected`), "unexpected");
    expect(() => assertOwnedSavedSession(saved, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/exactly the expected artifacts/i);
  });
});
