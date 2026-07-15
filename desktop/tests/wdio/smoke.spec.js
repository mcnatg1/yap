import { execFile } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

import {
  cleanupWdioSession,
  createTauriCapabilities,
  startWdioSession,
} from "@wdio/tauri-service";

const execFileAsync = promisify(execFile);
const specRoot = path.dirname(fileURLToPath(import.meta.url));
const desktopRoot = path.resolve(specRoot, "..", "..");
const binaryName = process.platform === "win32" ? "yap-desktop.exe" : "yap-desktop";
const appBinaryPath = process.env.APP_BINARY
  ?? path.join(desktopRoot, "src-tauri", "target", "debug", binaryName);

function requiredIsolationPath(name) {
  const value = process.env[name];
  if (!value || !path.isAbsolute(value)) {
    throw new Error(`${name} must be an absolute path for the native restart proof.`);
  }
  return value;
}

function restartSessionCapabilities(port, appDataRoot, liveRoot, modelsRoot, webviewRoot, pickerPath) {
  const capabilities = createTauriCapabilities(appBinaryPath, {
    driverProvider: "embedded",
    logLevel: "info",
    startTimeout: 60_000,
  });
  capabilities.browserName = "tauri";
  capabilities["wdio:tauriServiceOptions"] = {
    ...capabilities["wdio:tauriServiceOptions"],
    captureBackendLogs: false,
    captureFrontendLogs: false,
    driverProvider: "embedded",
    embeddedPort: port,
    env: {
      WEBVIEW2_USER_DATA_FOLDER: webviewRoot,
      YAP_APP_DATA_DIR: appDataRoot,
      YAP_LIVE_RECORDINGS_DIR: liveRoot,
      YAP_MODELS_DIR: modelsRoot,
      YAP_WDIO_PICKER_PATH: pickerPath,
      YAP_WDIO_RUN_ROOT: requiredIsolationPath("YAP_WDIO_RUN_ROOT"),
    },
    startTimeout: 60_000,
  };
  return capabilities;
}

async function invokeTauriCommandInSession(session, command, args = {}) {
  // The embedded service routes `tauri.execute` through one process-wide direct-eval port.
  // Programmatic restart sessions must use their own WebDriver connection instead.
  const result = await session.executeAsync((commandName, commandArgs, done) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (typeof invoke !== "function") {
      done({ error: "The session does not expose the Tauri invoke bridge.", ok: false });
      return;
    }
    invoke(commandName, commandArgs).then(
      (value) => done({ ok: true, value }),
      (error) => {
        let message;
        if (typeof error === "string") {
          message = error;
        } else {
          try {
            message = JSON.stringify(error);
          } catch {
            message = String(error);
          }
        }
        done({ error: message, ok: false });
      },
    );
  }, command, args);
  if (!result?.ok) {
    throw new Error(`Tauri command ${command} failed in the selected WebDriver session: ${result?.error}`);
  }
  return result.value;
}

async function processIdListeningOn(port) {
  const { stdout } = await execFileAsync("netstat.exe", ["-ano", "-p", "tcp"], {
    timeout: 5_000,
    windowsHide: true,
  });
  const listenerPattern = new RegExp(
    `^\\s*TCP\\s+\\S+:${port}\\s+\\S+\\s+LISTENING\\s+(\\d+)\\s*$`,
    "mi",
  );
  const processId = Number(stdout.match(listenerPattern)?.[1]);
  return Number.isInteger(processId) && processId > 0 ? processId : undefined;
}

async function findProcessIdListeningOn(port) {
  const processId = await processIdListeningOn(port);
  if (processId) return processId;
  throw new Error(`No native Yap process is listening on embedded WebDriver port ${port}.`);
}

function isProcessAlive(processId) {
  try {
    process.kill(processId, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    throw error;
  }
}

async function waitForProcessExit(processId, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!isProcessAlive(processId)) return;
    await delay(50);
  }
  throw new Error(`Native Yap process ${processId} remained alive after ${timeoutMs}ms.`);
}

function writeEmptyWaveFile(filePath) {
  const wave = Buffer.alloc(44);
  wave.write("RIFF", 0, "ascii");
  wave.writeUInt32LE(36, 4);
  wave.write("WAVE", 8, "ascii");
  wave.write("fmt ", 12, "ascii");
  wave.writeUInt32LE(16, 16);
  wave.writeUInt16LE(1, 20);
  wave.writeUInt16LE(1, 22);
  wave.writeUInt32LE(16_000, 24);
  wave.writeUInt32LE(32_000, 28);
  wave.writeUInt16LE(2, 32);
  wave.writeUInt16LE(16, 34);
  wave.write("data", 36, "ascii");
  wave.writeUInt32LE(0, 40);
  writeFileSync(filePath, wave, { flag: "wx" });
}

function comparableWindowsPath(candidate) {
  const withoutExtendedPrefix = candidate.startsWith("\\\\?\\")
    ? candidate.slice(4)
    : candidate;
  return path.win32.normalize(withoutExtendedPrefix).toLocaleLowerCase("en-US");
}

describe("Yap desktop shell", () => {
  it("opens the main window and exposes the WDIO Tauri bridge", async () => {
    await browser.tauri.switchWindow("main");
    await browser.pause(500);

    expect(typeof browser.tauri.execute).toBe("function");
    const bridge = await browser.execute(() => ({
      hasTauriInternals: typeof window.__TAURI_INTERNALS__?.invoke === "function",
      hasWdioTauri: typeof window.wdioTauri?.execute === "function",
    }));
    expect(bridge.hasTauriInternals).toBe(true);
    expect(bridge.hasWdioTauri).toBe(true);

    const heading = await $("h1");
    await heading.waitForDisplayed();
    expect(await heading.getText()).toContain("Welcome back");
  });

  it("keeps representative native command families registered", async () => {
    await browser.tauri.switchWindow("main");

    const commands = await browser.tauri.execute(async ({ core }) => ({
      history: await core.invoke("history_catalog"),
      live: await core.invoke("live_status"),
      server: await core.invoke("server_connection_status"),
      setup: await core.invoke("setup_status"),
    }));

    expect(typeof commands.setup.engineReady).toBe("boolean");
    expect(typeof commands.setup.engineStatus).toBe("string");
    expect([
      "not_set",
      "connecting",
      "ready",
      "offline",
      "sign_in_required",
      "retrying",
      "disabled",
    ]).toContain(commands.server.state);
    expect(typeof commands.server.capabilities.batchJobs).toBe("boolean");
    expect(typeof commands.server.capabilities.liveStreaming).toBe("boolean");
    expect(typeof commands.server.capabilities.jobStatus).toBe("boolean");
    expect(commands.server.checkedAtMs === null || typeof commands.server.checkedAtMs === "number").toBe(true);
    expect(commands.server.retryAtMs === null || typeof commands.server.retryAtMs === "number").toBe(true);
    expect(typeof commands.live.status).toBe("string");
    expect(typeof commands.live.visibility).toBe("string");
    expect(Array.isArray(commands.history.sessions)).toBe(true);
    expect(Array.isArray(commands.history.maintenanceWarnings)).toBe(true);
  });

  it("reports an enforced CSP violation for a disallowed remote script", async () => {
    await browser.tauri.switchWindow("main");
    const violation = await browser.executeAsync((done) => {
      const probeUrl = "https://example.invalid/yap-csp-probe.js";
      const script = document.createElement("script");
      let settled = false;

      const finish = (result) => {
        if (settled) return;
        settled = true;
        window.clearTimeout(timeout);
        document.removeEventListener("securitypolicyviolation", onViolation);
        script.remove();
        done(result);
      };
      const onViolation = (event) => {
        const blockedURI = String(event.blockedURI ?? "");
        if (!blockedURI.includes("yap-csp-probe")) return;
        finish({
          blockedURI,
          disposition: event.disposition,
          effectiveDirective: event.effectiveDirective,
        });
      };
      const timeout = window.setTimeout(
        () => finish({ error: "No securitypolicyviolation event was emitted." }),
        3_000,
      );

      document.addEventListener("securitypolicyviolation", onViolation);
      script.src = probeUrl;
      document.head.append(script);
    });

    expect(violation.error).toBeUndefined();
    expect(violation.blockedURI).toContain("yap-csp-probe.js");
    expect(["script-src", "script-src-elem"]).toContain(violation.effectiveDirective);
    expect(violation.disposition).toBe("enforce");
  });

  it("restores a Rust-owned queued job after a genuine native process restart", async function () {
    this.timeout(180_000);
    const runRoot = requiredIsolationPath("YAP_WDIO_RUN_ROOT");
    const proofRoot = path.join(runRoot, "native-restart-proof");
    const appDataRoot = path.join(proofRoot, "app-data");
    const liveRoot = path.join(proofRoot, "live-recordings");
    const modelsRoot = path.join(proofRoot, "models");
    const firstWebviewRoot = path.join(proofRoot, "webview-first");
    const secondWebviewRoot = path.join(proofRoot, "webview-second");
    const sourcePath = path.join(proofRoot, "restart-proof.wav");
    for (const directory of [
      proofRoot,
      appDataRoot,
      liveRoot,
      modelsRoot,
      firstWebviewRoot,
      secondWebviewRoot,
    ]) {
      mkdirSync(directory, { recursive: true });
    }
    writeEmptyWaveFile(sourcePath);

    const firstPort = 4455;
    const secondPort = 4456;
    let firstSession;
    let secondSession;
    let firstProcessId;
    let secondProcessId;
    try {
      firstSession = await startWdioSession(
        restartSessionCapabilities(firstPort, appDataRoot, liveRoot, modelsRoot, firstWebviewRoot, sourcePath),
      );
      await firstSession.switchToWindow("main");
      expect(await firstSession.getWindowHandle()).toBe("main");
      firstProcessId = await findProcessIdListeningOn(firstPort);
      const created = await invokeTauriCommandInSession(firstSession, "recording_jobs_pick_imports");
      expect(created).toHaveLength(1);
      expect(created[0].status).toBe("queued_server");
      expect(typeof created[0].id).toBe("string");
      expect(await firstSession.execute(() =>
        window.localStorage.getItem("yap.recordingQueue.v1"))).toBeNull();
      console.info(
        `[Task 7 restart] processA=${firstProcessId} job=${created[0].id} status=${created[0].status}`,
      );

      await cleanupWdioSession(firstSession);
      await waitForProcessExit(firstProcessId);
      firstSession = undefined;

      secondSession = await startWdioSession(
        restartSessionCapabilities(secondPort, appDataRoot, liveRoot, modelsRoot, secondWebviewRoot, sourcePath),
      );
      await secondSession.switchToWindow("main");
      expect(await secondSession.getWindowHandle()).toBe("main");
      secondProcessId = await findProcessIdListeningOn(secondPort);
      expect(secondProcessId).not.toBe(firstProcessId);

      const reopened = await invokeTauriCommandInSession(secondSession, "recording_jobs_snapshot");
      const restored = reopened.find((job) => job.id === created[0].id);
      expect(restored).toBeDefined();
      expect(restored.status).toBe("queued_server");
      expect(comparableWindowsPath(restored.sourcePath)).toBe(comparableWindowsPath(sourcePath));
      expect(await secondSession.execute(() =>
        window.localStorage.getItem("yap.recordingQueue.v1"))).toBeNull();
      console.info(
        `[Task 7 restart] processB=${secondProcessId} recovered=${restored.id} status=${restored.status}`,
      );
    } finally {
      if (secondSession) {
        await cleanupWdioSession(secondSession);
        if (secondProcessId) await waitForProcessExit(secondProcessId);
      }
      if (firstSession) {
        if (firstProcessId && !isProcessAlive(firstProcessId)) firstSession.sessionId = undefined;
        await cleanupWdioSession(firstSession);
        if (firstProcessId) await waitForProcessExit(firstProcessId);
      }
    }
  });
});
