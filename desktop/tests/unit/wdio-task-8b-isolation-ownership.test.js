import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import {
  attachWdioRunIsolation,
  createWdioRunIsolation,
  removePrivateRunRootWhenReleased,
} from "../wdio/task-8b-isolation.js";
import { installTask8bPrivateIsolationFixture } from "./wdio-task-8b-fixture.js";

const { privateIsolation } = installTask8bPrivateIsolationFixture();


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
    rmSync(runRoot, { recursive: true });
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
});
