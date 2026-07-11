import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import {
  MICROPHONE_PERMISSION_DENIED_PREFIX,
  assertOwnedSavedSession,
  assertRecordingRootEmpty,
  classifyNativeReadiness,
  createWdioRunIsolation,
  listRecordingArtifacts,
  registerTask8bLifecycleListeners,
  removePrivateRunRoot,
  removePrivateRunRootWhenReleased,
  resetPrivateRecordingRoot,
} from "../wdio/task-8b-helpers.js";

const temporaryRoots = [];

function temporaryRoot(label) {
  const root = path.join(
    tmpdir(),
    `yap-${label}-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`,
  );
  mkdirSync(root, { recursive: true });
  temporaryRoots.push(root);
  return root;
}

function writeCanonicalBundle(recordingRoot, name = "live-s-18c13f2a28c8be80-d018-2") {
  const suffixes = [
    ".capture.json",
    ".commit.json",
    ".transcript.r1.json",
    ".txt",
    ".wav",
  ];
  for (const suffix of suffixes) {
    writeFileSync(path.join(recordingRoot, `${name}${suffix}`), suffix === ".wav" ? Buffer.alloc(64) : "{}\n");
  }
  return {
    captureCommitPath: path.join(recordingRoot, `${name}.commit.json`),
    createdAtMs: 2_000,
    name,
    outputPath: path.join(recordingRoot, `${name}.txt`),
    sourcePath: path.join(recordingRoot, `${name}.wav`),
  };
}

afterEach(() => {
  while (temporaryRoots.length) {
    const root = temporaryRoots.pop();
    if (existsSync(root)) {
      rmSync(root, { force: true, recursive: true });
    }
  }
});

describe("Task 8b native readiness classification", () => {
  const ready = {
    deviceError: null,
    devices: [{ id: "0:Microphone" }],
    model: { status: "ready" },
    preflight: { error: null, status: "idle" },
  };

  it.each(["missing", "disabled", "corrupted"])("skips only the precise %s model state", (status) => {
    expect(classifyNativeReadiness({ ...ready, model: { status } })).toMatchObject({
      action: "skip",
    });
  });

  it("skips a successful enumeration that returns zero devices", () => {
    expect(classifyNativeReadiness({ ...ready, devices: [], preflight: null })).toEqual({
      action: "skip",
      reason: "no input device was enumerated",
    });
  });

  it("skips only a native error carrying the narrow permission marker", () => {
    const permissionError = `${MICROPHONE_PERMISSION_DENIED_PREFIX} Access is denied. (0x80070005)`;
    expect(classifyNativeReadiness({ ...ready, deviceError: permissionError, devices: null })).toMatchObject({
      action: "skip",
    });
    expect(classifyNativeReadiness({
      ...ready,
      preflight: { error: permissionError, status: "blocked" },
    })).toMatchObject({ action: "skip" });
  });

  it.each([
    "Microphone device enumeration failed: backend unavailable",
    "Microphone default input configuration failed: unsupported format",
    "Microphone input stream build failed: backend regression",
    "Microphone input stream playback failed: device invalidated",
    "Microphone access failed: generic legacy prefix",
    "Permission denied",
  ])("fails instead of skipping native regression: %s", (error) => {
    expect(() => classifyNativeReadiness({ ...ready, deviceError: error, devices: null }))
      .toThrow(/enumeration failed/i);
    expect(() => classifyNativeReadiness({
      ...ready,
      preflight: { error, status: "blocked" },
    })).toThrow(/preflight failure/i);
  });

  it("fails unknown blocked and no-sample preflights", () => {
    expect(() => classifyNativeReadiness({
      ...ready,
      preflight: { error: "No input detected.", status: "blocked" },
    })).toThrow(/preflight failure/i);
    expect(() => classifyNativeReadiness({
      ...ready,
      preflight: { error: null, status: "blocked" },
    })).toThrow(/unknown error/i);
  });

  it.each(["missing", "disabled", "corrupted"])(
    "does not let the %s model skip hide a simultaneous native regression",
    (status) => {
      expect(() => classifyNativeReadiness({
        ...ready,
        deviceError: "Microphone device enumeration failed: backend unavailable",
        devices: null,
        model: { status },
      })).toThrow(/enumeration failed/i);
      expect(() => classifyNativeReadiness({
        ...ready,
        model: { status },
        preflight: {
          error: "Microphone input stream build failed: backend regression",
          status: "blocked",
        },
      })).toThrow(/preflight failure/i);
    },
  );
});

describe("Task 8b transactional lifecycle listeners", () => {
  it("immediately unregisters earlier listeners when partial setup fails", async () => {
    const calls = [];
    const unlisten = () => calls.push("unlisten-live-session");
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          calls.push(name);
          if (name === "live-level") throw new Error("registration failed");
          return unlisten;
        },
      },
    };

    try {
      await expect(registerTask8bLifecycleListeners()).rejects.toThrow("registration failed");
      expect(calls).toEqual(["live-session", "live-level", "unlisten-live-session"]);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("unregisters all listeners exactly once", async () => {
    const unlistened = [];
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          return () => unlistened.push(name);
        },
      },
    };

    try {
      await expect(registerTask8bLifecycleListeners()).resolves.toBe(3);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(3);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(0);
      expect(unlistened).toEqual(["live-session", "live-level", "live-session-saved"]);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });
});

describe("Task 8b canonical saved-session ownership", () => {
  function fixture() {
    const recordingRoot = temporaryRoot("owned-recordings");
    return { recordingRoot, saved: writeCanonicalBundle(recordingRoot) };
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
    const { recordingRoot, saved } = fixture();
    expect(() => assertOwnedSavedSession({ ...saved, sourcePath: `${saved.name}.wav` }, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/absolute Windows path/i);

    const foreignRoot = temporaryRoot("foreign-recordings");
    const foreignWav = path.join(foreignRoot, `${saved.name}.wav`);
    writeFileSync(foreignWav, Buffer.alloc(64));
    expect(() => assertOwnedSavedSession({ ...saved, sourcePath: foreignWav }, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/isolated recording root/i);
  });

  it("rejects a canonical path escape even when the lexical parent matches", () => {
    const { recordingRoot, saved } = fixture();
    const foreignRoot = temporaryRoot("canonical-escape");
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
    })).toThrow(/name\/session relationship/i);

    writeFileSync(path.join(recordingRoot, `${saved.name}.unexpected`), "unexpected");
    expect(() => assertOwnedSavedSession(saved, recordingRoot, {
      nowMs: 2_500,
      runStartedAtMs: 1_500,
    })).toThrow(/exactly the expected artifacts/i);
  });
});

describe("Task 8b WDIO run isolation", () => {
  it("sets absolute recording and WebView roots under one generated temp run", () => {
    const testsRoot = temporaryRoot("wdio-tests");
    const env = {};
    const isolation = createWdioRunIsolation(testsRoot, env, {
      token: "20260711-001122334455",
    });

    expect(path.win32.isAbsolute(isolation.recordingRoot)).toBe(true);
    expect(path.win32.isAbsolute(isolation.webviewRoot)).toBe(true);
    expect(path.dirname(isolation.recordingRoot)).toBe(isolation.runRoot);
    expect(path.dirname(isolation.webviewRoot)).toBe(isolation.runRoot);
    expect(env.YAP_LIVE_RECORDINGS_DIR).toBe(isolation.recordingRoot);
    expect(env.WEBVIEW2_USER_DATA_FOLDER).toBe(isolation.webviewRoot);
    expect(listRecordingArtifacts(isolation.recordingRoot)).toEqual([]);

    removePrivateRunRoot(isolation);
    expect(existsSync(isolation.runRoot)).toBe(false);
  });

  it("resets only the proven private recording root and refuses an outside cleanup", () => {
    const testsRoot = temporaryRoot("wdio-cleanup");
    const isolation = createWdioRunIsolation(testsRoot, {}, { token: "safe-cleanup" });
    writeFileSync(path.join(isolation.recordingRoot, "owned.txt"), "owned");
    expect(resetPrivateRecordingRoot(isolation)).toEqual(["owned.txt"]);
    assertRecordingRootEmpty(isolation.recordingRoot);

    const outside = temporaryRoot("must-survive");
    const marker = path.join(outside, "marker.txt");
    writeFileSync(marker, "keep");
    expect(() => removePrivateRunRoot({ ...isolation, runRoot: outside })).toThrow(/refusing/i);
    expect(existsSync(marker)).toBe(true);
  });

  it("removes the private run root through the asynchronous service-cleanup seam", async () => {
    const testsRoot = temporaryRoot("wdio-async-cleanup");
    const isolation = createWdioRunIsolation(testsRoot, {}, { token: "async-cleanup" });

    await removePrivateRunRootWhenReleased(isolation);

    expect(existsSync(isolation.runRoot)).toBe(false);
  });
});
