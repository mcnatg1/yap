import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import {
  lstatSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";

import { matchCompletedRemoteTranscript } from "./phase5-gate-support.js";

const tunnelHost = "127.0.0.1";
const tunnelPort = 18765;
let tunnelProcess;

function requireEnvironment(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required for the Phase 5 gate.`);
  return value;
}

async function invoke(command, args = {}) {
  const result = await browser.executeAsync((commandName, commandArgs, done) => {
    const invokeCommand = window.__TAURI__?.core?.invoke;
    if (typeof invokeCommand !== "function") {
      done({ error: "Tauri invoke bridge unavailable", ok: false });
      return;
    }
    invokeCommand(commandName, commandArgs).then(
      (value) => done({ ok: true, value }),
      (error) => done({
        error: typeof error === "object" && error && "code" in error
          ? String(error.code)
          : "native command failed",
        ok: false,
      }),
    );
  }, command, args);
  if (!result?.ok) {
    throw new Error(`Tauri command ${command} failed: ${result?.error ?? "unknown error"}`);
  }
  return result.value;
}

function canonicalPath(value) {
  return path.resolve(realpathSync.native(value));
}

function requireSshAlias() {
  const alias = requireEnvironment("YAP_PHASE5_GATE_SSH_ALIAS");
  if (!/^[A-Za-z0-9._-]+$/.test(alias)) {
    throw new Error("YAP_PHASE5_GATE_SSH_ALIAS must be one explicit SSH config alias.");
  }
  return alias;
}

async function healthIsReachable() {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 750);
  try {
    const response = await fetch(`http://${tunnelHost}:${tunnelPort}/v1/health`, {
      cache: "no-store",
      redirect: "error",
      signal: controller.signal,
    });
    return response.ok;
  } catch {
    return false;
  } finally {
    clearTimeout(timeout);
  }
}

async function waitForHealth(expected, child, label) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (child && child.exitCode !== null) {
      throw new Error(`The Phase 5 SSH forward exited before ${label}.`);
    }
    if (await healthIsReachable() === expected) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`The Phase 5 SSH forward did not ${label} within 15 seconds.`);
}

async function startTunnel(alias) {
  if (await healthIsReachable()) {
    throw new Error("Port 18765 was already reachable before the gate-owned SSH forward.");
  }
  const child = spawn(
    "ssh.exe",
    [
      "-o", "BatchMode=yes",
      "-o", "ExitOnForwardFailure=yes",
      "-o", "ServerAliveInterval=15",
      "-o", "ServerAliveCountMax=3",
      "-N", "-T",
      "-L", `${tunnelHost}:${tunnelPort}:${tunnelHost}:${tunnelPort}`,
      alias,
    ],
    { stdio: "ignore", windowsHide: true },
  );
  child.once("error", () => {});
  try {
    await waitForHealth(true, child, "become reachable");
  } catch (error) {
    if (child.exitCode === null) child.kill();
    throw error;
  }
  return child;
}

async function stopTunnel(child) {
  if (child.exitCode === null) {
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error("The Phase 5 SSH forward did not stop within 10 seconds.")),
        10_000,
      );
      child.once("exit", () => {
        clearTimeout(timeout);
        resolve();
      });
      if (!child.kill()) {
        clearTimeout(timeout);
        reject(new Error("The Phase 5 gate could not stop its SSH forward."));
      }
    });
  }
  await waitForHealth(false, undefined, "become unreachable");
}

describe("Phase 5 checked-head private-server gate", () => {
  before(async () => {
    tunnelProcess = await startTunnel(requireSshAlias());
  });

  after(async () => {
    if (tunnelProcess) {
      const owned = tunnelProcess;
      tunnelProcess = undefined;
      await stopTunnel(owned);
    }
  });

  it("imports through the real tunneled GB10 worker and opens the verified History result", async () => {
    await browser.tauri.switchWindow("main");

    const checkedHead = requireEnvironment("YAP_CHECKED_HEAD");
    const expectedOrigin = requireEnvironment("YAP_PHASE5_GATE_BASE_URL");
    const evidenceDirectory = requireEnvironment("YAP_PHASE5_GATE_EVIDENCE_DIR");
    const fixturePath = requireEnvironment("YAP_WDIO_PICKER_PATH");
    const fixtureSha256 = requireEnvironment("YAP_PHASE5_GATE_FIXTURE_SHA256");
    const expectedModelId = requireEnvironment("YAP_PHASE5_GATE_MODEL_ID");
    const expectedModelRevision = requireEnvironment("YAP_PHASE5_GATE_MODEL_REVISION");
    const appDataRoot = canonicalPath(requireEnvironment("YAP_APP_DATA_DIR"));
    const timeoutMs = Number(requireEnvironment("YAP_PHASE5_GATE_TIMEOUT_MS"));

    const settings = await invoke("server_settings");
    expect(settings).toEqual({ schemaVersion: 1, enabled: true, baseUrl: expectedOrigin });
    const connection = await invoke("refresh_server_connection");
    expect(connection.state).toBe("ready");
    expect(connection.capabilities).toEqual({
      batchJobs: true,
      jobStatus: true,
      liveStreaming: false,
    });

    const created = await invoke("recording_jobs_pick_imports");
    expect(created).toHaveLength(1);
    expect(created[0].status).toBe("queued_server");
    expect(canonicalPath(created[0].sourcePath)).toBe(canonicalPath(fixturePath));
    const createdJob = created[0];
    const clientJobId = createdJob.id;
    const observedStatuses = new Set([createdJob.status]);
    let history;
    let terminalFailure;

    const interruptedTunnel = tunnelProcess;
    tunnelProcess = undefined;
    await stopTunnel(interruptedTunnel);
    const interruptedConnection = await invoke("refresh_server_connection");
    expect(interruptedConnection.state).toBe("retrying");
    expect((await invoke("server_settings")).baseUrl).toBe(expectedOrigin);
    const interruptedSnapshot = await invoke("recording_jobs_snapshot");
    const interruptedJob = interruptedSnapshot.find((candidate) => candidate.id === clientJobId);
    expect(interruptedJob).toBeDefined();
    expect(["queued_server", "preprocessing", "uploading", "server_processing"])
      .toContain(interruptedJob.status);
    observedStatuses.add(interruptedJob.status);

    tunnelProcess = await startTunnel(requireSshAlias());
    const restoredConnection = await invoke("refresh_server_connection");
    expect(restoredConnection.state).toBe("ready");
    expect((await invoke("server_settings")).baseUrl).toBe(expectedOrigin);

    await browser.waitUntil(
      async () => {
        const snapshot = await invoke("recording_jobs_snapshot");
        const job = snapshot.find((candidate) => candidate.id === clientJobId);
        if (job) {
          observedStatuses.add(job.status);
        }
        if (job && ["failed", "cancelled"].includes(job.status)) {
          terminalFailure = new Error(
            `The Phase 5 job reached ${job.status} (${job.error ?? "no private-safe error projection"}).`,
          );
          return true;
        }
        const catalog = await invoke("recording_jobs_completed_transcripts");
        if (catalog.maintenanceWarnings.length > 0) {
          terminalFailure = new Error(
            `The Phase 5 History catalog reported maintenance warnings: ${catalog.maintenanceWarnings.join("; ")}`,
          );
          return true;
        }
        history = matchCompletedRemoteTranscript(createdJob, catalog);
        return Boolean(history);
      },
      {
        interval: 1_000,
        timeout: timeoutMs,
        timeoutMsg: "The checked-head Phase 5 job did not complete within the gate timeout.",
      },
    );

    if (terminalFailure) throw terminalFailure;
    expect(createdJob.route).toBe("serverBatch");
    expect(history).toBeDefined();
    expect(history.warning).toBeUndefined();
    expect(canonicalPath(history.sourcePath)).toBe(canonicalPath(fixturePath));

    const transcriptPath = canonicalPath(history.outputPath);
    const remoteRoot = path.join(appDataRoot, "remote-jobs");
    const transcriptRelative = path.relative(remoteRoot, transcriptPath);
    expect(transcriptRelative.startsWith("..")).toBe(false);
    expect(path.isAbsolute(transcriptRelative)).toBe(false);
    expect(lstatSync(transcriptPath).isSymbolicLink()).toBe(false);
    expect(statSync(transcriptPath).isFile()).toBe(true);

    const resultPath = path.join(path.dirname(transcriptPath), "result.json");
    const resultMetadata = lstatSync(resultPath);
    expect(resultMetadata.isSymbolicLink()).toBe(false);
    expect(resultMetadata.isFile()).toBe(true);
    const resultBytes = readFileSync(resultPath);
    const result = JSON.parse(resultBytes.toString("utf8"));
    expect(result.sessionId).toBe(history.sessionId);
    expect(result.revision).toBe(1);
    expect(result.authority).toBe("server_authoritative");
    expect(result.status).toBe("complete");
    expect(result.transcript.trim().length).toBeGreaterThan(0);
    expect(result.modelProvenance).toContainEqual({
      calibrationRevision: "asr-not-applicable",
      modelId: expectedModelId,
      revision: expectedModelRevision,
    });

    await browser.waitUntil(
      async () => browser.execute(
        (name, expectedTranscript) => {
          const row = [...document.querySelectorAll("[data-history-entry-row]")]
            .find((candidate) => candidate.textContent?.includes(name));
          const dialog = document.querySelector('[role="dialog"]');
          const transcript = dialog?.querySelector("pre")?.textContent?.trim() ?? "";
          return Boolean(row && dialog && transcript === expectedTranscript);
        },
        history.name,
        result.transcript.trim(),
      ),
      {
        interval: 250,
        timeout: 15_000,
        timeoutMsg: "History did not open the verified server-authoritative transcript.",
      },
    );

    writeFileSync(
      path.join(evidenceDirectory, "native-vertical-slice.json"),
      `${JSON.stringify({
        schemaVersion: 1,
        checkedHead,
        fixtureSha256,
        clientJobId,
        clientRoute: createdJob.route,
        serverOrigin: expectedOrigin,
        sessionId: result.sessionId,
        resultRevision: result.revision,
        resultAuthority: result.authority,
        resultArtifactSha256: createHash("sha256").update(resultBytes).digest("hex"),
        transcriptBytes: Buffer.byteLength(result.transcript, "utf8"),
        modelProvenance: result.modelProvenance,
        observedStatuses: [...observedStatuses].sort(),
        tunnelInterruptionState: interruptedConnection.state,
        tunnelRestoredState: restoredConnection.state,
        immutableJobSurvivedTunnelInterruption: true,
        historyOpenedVerifiedResult: true,
        status: "passed",
      }, null, 2)}\n`,
      { encoding: "utf8", flag: "wx" },
    );
  });
});
