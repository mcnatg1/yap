import {
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  rmdirSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { rm as removeAsync } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import {
  assertRecordingRootEmpty,
  listRecordingArtifacts,
  removePrivateRunRootWhenReleased,
  resetPrivateRecordingRoot,
} from "../wdio/task-8b-helpers.js";
import { installTask8bPrivateIsolationFixture } from "./wdio-task-8b-fixture.js";

const { privateIsolation } = installTask8bPrivateIsolationFixture();


describe("Task 8b WDIO private cleanup", () => {
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

  it("preserves the ownership marker while a private child is locked", async () => {
    const isolation = privateIsolation("locked-child-marker");
    const ownerMarker = path.join(isolation.runRoot, ".yap-wdio-run-owner");
    let blockedWebviewOnce = false;
    let observedRetry = false;

    try {
      await removePrivateRunRootWhenReleased(isolation, {
        maxAttempts: 3,
        onRetry() {
          observedRetry = true;
          expect(existsSync(ownerMarker)).toBe(true);
        },
        removeDirectory: async (target, options) => {
          if (target === isolation.runRoot) {
            const nonMarkerChildren = readdirSync(target)
              .filter((name) => name !== ".yap-wdio-run-owner");
            if (nonMarkerChildren.length > 0) {
              unlinkSync(ownerMarker);
              const error = new Error("recursive removal deleted the marker before a child released");
              error.code = "EBUSY";
              throw error;
            }
          }
          if (target === isolation.webviewRoot && !blockedWebviewOnce) {
            blockedWebviewOnce = true;
            const error = new Error("WebView directory busy");
            error.code = "EBUSY";
            throw error;
          }
          await removeAsync(target, options);
        },
        retryDelayMs: 0,
      });
    } finally {
      if (existsSync(isolation.runRoot) && !existsSync(ownerMarker)) {
        writeFileSync(ownerMarker, `${isolation.token}\n`, { flag: "wx" });
      }
    }

    expect(blockedWebviewOnce).toBe(true);
    expect(observedRetry).toBe(true);
    expect(existsSync(isolation.runRoot)).toBe(false);
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
