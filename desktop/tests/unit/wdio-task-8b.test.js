import { execFileSync } from "node:child_process";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  rmdirSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

import {
  MICROPHONE_PERMISSION_DENIED_PREFIX,
  attachWdioRunIsolation,
  assertOwnedSavedSession,
  assertRecordingRootEmpty,
  classifyNativeReadiness,
  createWdioRunIsolation,
  listRecordingArtifacts,
  registerTask8bLifecycleListeners,
  removePrivateRunRootWhenReleased,
  resetPrivateRecordingRoot,
  waitForTask8bSavedEvent,
} from "../wdio/task-8b-helpers.js";

const privateIsolations = [];
let fixtureSequence = 0;

function privateIsolation(label, env = {}) {
  fixtureSequence += 1;
  const safeLabel = label.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  const isolation = createWdioRunIsolation(env, {
    token: `unit-${safeLabel}-${process.pid}-${Date.now()}-${fixtureSequence}`,
  });
  privateIsolations.push(isolation);
  return isolation;
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
    sessionId: name.slice("live-".length),
    sourcePath: path.join(recordingRoot, `${name}.wav`),
  };
}

afterEach(async () => {
  const failures = [];
  while (privateIsolations.length) {
    const isolation = privateIsolations.pop();
    try {
      await removePrivateRunRootWhenReleased(isolation);
    } catch (error) {
      failures.push(error);
    }
  }
  if (failures.length > 0) {
    throw new AggregateError(failures, "Task 8b unit fixture cleanup failed");
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

  it("retains a rejecting unlistener for a later successful retry", async () => {
    let rejectOnceAttempts = 0;
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          if (name !== "live-session") return () => undefined;
          return async () => {
            rejectOnceAttempts += 1;
            if (rejectOnceAttempts === 1) throw new Error("unlisten retry required");
          };
        },
      },
    };

    try {
      await registerTask8bLifecycleListeners();
      await expect(globalThis.__yapTask8bLifecycle.cleanup())
        .rejects.toThrow("unlisten retry required");
      expect(globalThis.__yapTask8bLifecycle.unlisteners).toHaveLength(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(0);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("preserves failed registration cleanup handles for outer-finally recovery", async () => {
    let cleanupAttempts = 0;
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          if (name === "live-level") throw new Error("registration failed");
          return async () => {
            cleanupAttempts += 1;
            if (cleanupAttempts === 1) throw new Error("partial cleanup failed");
          };
        },
      },
    };

    try {
      await expect(registerTask8bLifecycleListeners())
        .rejects.toThrow(/registration failed.*partial cleanup failed/i);
      expect(globalThis.__yapTask8bLifecycle.unlisteners).toHaveLength(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(0);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("waits through delayed saved-event dispatch before returning evidence", async () => {
    globalThis.__yapTask8bLifecycle = { levels: [], saved: [], sessions: [] };
    const dispatch = setTimeout(() => {
      globalThis.__yapTask8bLifecycle.saved.push({ name: "live-s-1-2-3" });
    }, 10);

    try {
      await expect(waitForTask8bSavedEvent({}, {
        expectedCount: 1,
        pollIntervalMs: 1,
        timeoutMs: 100,
      })).resolves.toMatchObject({
        saved: [{ name: "live-s-1-2-3" }],
      });
    } finally {
      clearTimeout(dispatch);
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("fails within the bounded saved-event deadline", async () => {
    globalThis.__yapTask8bLifecycle = { levels: [], saved: [], sessions: [] };
    try {
      await expect(waitForTask8bSavedEvent({}, {
        expectedCount: 1,
        pollIntervalMs: 1,
        timeoutMs: 10,
      })).rejects.toThrow(/timed out waiting for 1 saved event/i);
    } finally {
      delete globalThis.__yapTask8bLifecycle;
    }
  });
});

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

describe("Task 8b WDIO run isolation", () => {
  const windowsIt = process.platform === "win32" ? it : it.skip;

  windowsIt("polls native main-window recovery without weakening uniqueness", () => {
    const recoveryTest = fileURLToPath(
      new URL("../scripts/native-window-recovery.test.ps1", import.meta.url),
    );
    const output = execFileSync(
      "pwsh.exe",
      ["-NoProfile", "-NonInteractive", "-File", recoveryTest],
      { encoding: "utf8", windowsHide: true },
    );

    expect(output).toContain("Native WDIO window-recovery tests passed.");
  });

  it("attaches a worker to the launcher-owned isolation without reclaiming it", () => {
    const env = {};
    const isolation = privateIsolation("worker-attach", env);
    const ownerMarker = path.join(isolation.runRoot, ".yap-wdio-run-owner");
    const markerBefore = readFileSync(ownerMarker, "utf8");
    env.WDIO_WORKER_ID = "0-0";

    expect(attachWdioRunIsolation(env)).toEqual(isolation);
    expect(readFileSync(ownerMarker, "utf8")).toBe(markerBefore);
  });

  it("rejects a worker path substitution without disturbing launcher ownership", () => {
    const env = {};
    const isolation = privateIsolation("worker-substitution", env);
    const ownerMarker = path.join(isolation.runRoot, ".yap-wdio-run-owner");

    expect(() => attachWdioRunIsolation({
      ...env,
      YAP_MODELS_DIR: path.join(isolation.runRoot, "substituted-models"),
    })).toThrow(/models.*exact|inherited.*models|does not match/i);
    expect(readFileSync(ownerMarker, "utf8")).toBe(`${isolation.token}\n`);
    expect(existsSync(isolation.runRoot)).toBe(true);
  });

  it("rejects workers missing the inherited token or a required path", () => {
    const env = {};
    privateIsolation("worker-missing-env", env);

    expect(() => attachWdioRunIsolation({
      ...env,
      YAP_WDIO_RUN_TOKEN: "",
    })).toThrow(/token/i);
    const { YAP_MODELS_DIR: _missing, ...missingModels } = env;
    expect(() => attachWdioRunIsolation(missingModels)).toThrow(/models/i);
  });

  it("rejects an inherited run root before claiming it", () => {
    const token = `unit-inherited-${process.pid}-${Date.now()}`;
    const tempRoot = path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      "..",
      "results",
      "temp",
      "wdio",
    );
    const runRoot = path.join(tempRoot, `run-${token}`);
    mkdirSync(runRoot, { recursive: true });
    writeFileSync(path.join(runRoot, "must-survive.txt"), "keep");

    expect(() => createWdioRunIsolation({}, { token })).toThrow(/already exists|inherited|stale|colliding/i);
    expect(existsSync(path.join(runRoot, "must-survive.txt"))).toBe(true);
    rmdirSync(runRoot, { recursive: true });
  });

  it("rejects cleanup when the exclusive ownership marker is missing or mismatched", async () => {
    const isolation = privateIsolation("owner-marker");
    const ownerMarker = path.join(isolation.runRoot, ".yap-wdio-run-owner");
    const original = readFileSync(ownerMarker, "utf8");
    unlinkSync(ownerMarker);

    await expect(removePrivateRunRootWhenReleased(isolation)).rejects.toThrow(/ownership marker/i);
    expect(existsSync(isolation.runRoot)).toBe(true);

    writeFileSync(ownerMarker, `${original.trim()}-foreign\n`, { flag: "wx" });
    await expect(removePrivateRunRootWhenReleased(isolation)).rejects.toThrow(/ownership marker/i);
    expect(existsSync(isolation.runRoot)).toBe(true);
    writeFileSync(ownerMarker, original, { flag: "w" });
  });

  it("rejects paired temp-root and run-root substitution", async () => {
    const owner = privateIsolation("paired-substitution");
    const tempRoot = path.join(owner.runRoot, "outside");
    const runRoot = path.join(tempRoot, "run-victim");
    const recordingRoot = path.join(runRoot, "live-recordings");
    const webviewRoot = path.join(runRoot, "webview2");
    mkdirSync(recordingRoot, { recursive: true });
    mkdirSync(webviewRoot);
    const marker = path.join(runRoot, "marker.txt");
    writeFileSync(marker, "keep");

    await expect(removePrivateRunRootWhenReleased({
      recordingRoot,
      runRoot,
      tempRoot,
      token: "victim",
      webviewRoot,
    })).rejects.toThrow(/fixed WDIO temp root/i);
    expect(existsSync(marker)).toBe(true);
  });

  it("rejects a token and run-root mismatch", async () => {
    const isolation = privateIsolation("token-mismatch");
    const marker = path.join(isolation.runRoot, "marker.txt");
    writeFileSync(marker, "keep");

    await expect(removePrivateRunRootWhenReleased({
      ...isolation,
      token: "different-token",
    })).rejects.toThrow(/token.*run root/i);
    expect(existsSync(marker)).toBe(true);
  });

  it.each(["../escape", "trailing-", "UPPERCASE"])(
    "rejects the non-canonical run token %s",
    (token) => {
      expect(() => createWdioRunIsolation({}, { token })).toThrow(/unsupported characters/i);
    },
  );

  it("rejects a substituted recording-root child", async () => {
    const isolation = privateIsolation("recording-substitution");
    const substituted = path.join(isolation.runRoot, "different-recordings");
    mkdirSync(substituted);
    const marker = path.join(substituted, "marker.txt");
    const runMarker = path.join(isolation.runRoot, "run-marker.txt");
    writeFileSync(marker, "keep");
    writeFileSync(runMarker, "keep");

    await expect(resetPrivateRecordingRoot({
      ...isolation,
      recordingRoot: substituted,
    })).rejects.toThrow(/recording root.*exact child/i);
    expect(existsSync(marker)).toBe(true);

    await expect(removePrivateRunRootWhenReleased({
      ...isolation,
      webviewRoot: substituted,
    })).rejects.toThrow(/WebView root.*exact child/i);
    await expect(removePrivateRunRootWhenReleased({
      ...isolation,
      appDataRoot: substituted,
    })).rejects.toThrow(/app-data root.*exact child/i);
    await expect(removePrivateRunRootWhenReleased({
      ...isolation,
      modelsRoot: substituted,
    })).rejects.toThrow(/models root.*exact child/i);
    expect(existsSync(runMarker)).toBe(true);
  });

  it("rejects an exact recording-root reparse redirect when links are supported", async () => {
    const isolation = privateIsolation("recording-link");
    const redirectTarget = path.join(isolation.runRoot, "redirect-target");
    mkdirSync(redirectTarget);
    rmdirSync(isolation.recordingRoot);
    try {
      symlinkSync(redirectTarget, isolation.recordingRoot, "junction");
    } catch (error) {
      if (["EPERM", "ENOTSUP", "UNKNOWN"].includes(error?.code)) {
        mkdirSync(isolation.recordingRoot);
        return;
      }
      throw error;
    }

    try {
      await expect(resetPrivateRecordingRoot(isolation)).rejects.toThrow(/link|reparse|redirect/i);
    } finally {
      if (existsSync(isolation.recordingRoot) && lstatSync(isolation.recordingRoot).isSymbolicLink()) {
        unlinkSync(isolation.recordingRoot);
        mkdirSync(isolation.recordingRoot);
      }
    }
  });

  it("revalidates relationships after an asynchronous removal retry", async () => {
    const isolation = privateIsolation("retry-revalidation");
    const redirectTarget = path.join(isolation.runRoot, "retry-redirect-target");
    mkdirSync(redirectTarget);
    let removeCalls = 0;
    let linkSupported = true;

    try {
      await expect(removePrivateRunRootWhenReleased(isolation, {
        maxAttempts: 3,
        onRetry() {
          rmdirSync(isolation.recordingRoot);
          try {
            symlinkSync(redirectTarget, isolation.recordingRoot, "junction");
          } catch (error) {
            linkSupported = false;
            mkdirSync(isolation.recordingRoot);
            if (!["EPERM", "ENOTSUP", "UNKNOWN"].includes(error?.code)) throw error;
          }
        },
        removeDirectory: async () => {
          removeCalls += 1;
          const error = new Error("directory busy");
          error.code = "EBUSY";
          throw error;
        },
        retryDelayMs: 0,
      })).rejects.toThrow(linkSupported ? /link|reparse|redirect/i : /locked/i);
      if (linkSupported) expect(removeCalls).toBe(1);
    } finally {
      if (existsSync(isolation.recordingRoot) && lstatSync(isolation.recordingRoot).isSymbolicLink()) {
        unlinkSync(isolation.recordingRoot);
        mkdirSync(isolation.recordingRoot);
      }
    }
  });

  it("sets exact absolute roots below the module-derived fixed temp parent", async () => {
    const env = {};
    const isolation = privateIsolation("fixed-layout", env);
    const expectedTempRoot = path.resolve(
      path.dirname(fileURLToPath(import.meta.url)),
      "..",
      "results",
      "temp",
      "wdio",
    );

    expect(isolation.tempRoot).toBe(expectedTempRoot);
    expect(isolation.runRoot).toBe(path.join(expectedTempRoot, `run-${isolation.token}`));
    expect(path.win32.isAbsolute(isolation.appDataRoot)).toBe(true);
    expect(path.win32.isAbsolute(isolation.modelsRoot)).toBe(true);
    expect(path.win32.isAbsolute(isolation.recordingRoot)).toBe(true);
    expect(path.win32.isAbsolute(isolation.webviewRoot)).toBe(true);
    expect(path.dirname(isolation.appDataRoot)).toBe(isolation.runRoot);
    expect(path.dirname(isolation.modelsRoot)).toBe(isolation.runRoot);
    expect(path.dirname(isolation.recordingRoot)).toBe(isolation.runRoot);
    expect(path.dirname(isolation.webviewRoot)).toBe(isolation.runRoot);
    expect(env.YAP_APP_DATA_DIR).toBe(isolation.appDataRoot);
    expect(env.YAP_MODELS_DIR).toBe(isolation.modelsRoot);
    expect(env.YAP_LIVE_RECORDINGS_DIR).toBe(isolation.recordingRoot);
    expect(env.WEBVIEW2_USER_DATA_FOLDER).toBe(isolation.webviewRoot);
    expect(listRecordingArtifacts(isolation.recordingRoot)).toEqual([]);

    await removePrivateRunRootWhenReleased(isolation);
    expect(existsSync(isolation.runRoot)).toBe(false);
  });

  it("resets only the proven private recording root and refuses an outside cleanup", async () => {
    const isolation = privateIsolation("safe-cleanup");
    writeFileSync(path.join(isolation.recordingRoot, "owned.txt"), "owned");
    await expect(resetPrivateRecordingRoot(isolation)).resolves.toEqual(["owned.txt"]);
    assertRecordingRootEmpty(isolation.recordingRoot);

    const outside = privateIsolation("must-survive");
    const marker = path.join(outside.runRoot, "marker.txt");
    writeFileSync(marker, "keep");
    await expect(removePrivateRunRootWhenReleased({
      ...isolation,
      runRoot: outside.runRoot,
    })).rejects.toThrow(/token.*run root/i);
    expect(existsSync(marker)).toBe(true);
  });

  it("removes the private run root through the asynchronous service-cleanup seam", async () => {
    const isolation = privateIsolation("async-cleanup");

    await removePrivateRunRootWhenReleased(isolation);

    expect(existsSync(isolation.runRoot)).toBe(false);
  });
});
