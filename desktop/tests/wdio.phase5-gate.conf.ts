import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  constants,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { config as baseConfig } from "./wdio.conf.ts";
import {
  resolvePhase5GateTimeout,
  sameWindowsPath,
} from "./wdio/phase5-gate-support.js";

const testsRoot = path.dirname(fileURLToPath(import.meta.url));
const desktopRoot = path.resolve(testsRoot, "..");
const repoRoot = path.resolve(desktopRoot, "..");
const checkedHead = process.env.YAP_CHECKED_HEAD ?? "";
const baseUrl = process.env.YAP_PHASE5_GATE_BASE_URL ?? "";
const evidenceDirectory = process.env.YAP_PHASE5_GATE_EVIDENCE_DIR ?? "";
const worker = Boolean(process.env.WDIO_WORKER_ID);

function requireCheckedHead() {
  if (!/^[0-9a-f]{40}$/.test(checkedHead)) {
    throw new Error("YAP_CHECKED_HEAD must be the exact lowercase Phase 5 candidate SHA.");
  }
  const actualHead = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
  }).trim();
  if (actualHead !== checkedHead) {
    throw new Error("YAP_CHECKED_HEAD does not match the checked-out repository HEAD.");
  }
  const status = execFileSync(
    "git",
    ["status", "--porcelain=v1", "--untracked-files=normal"],
    { cwd: repoRoot, encoding: "utf8" },
  ).trim();
  if (status) {
    throw new Error("The Phase 5 native gate requires a clean checked head.");
  }
}

function requireLoopbackGateOrigin() {
  let parsed: URL;
  try {
    parsed = new URL(baseUrl);
  } catch {
    throw new Error("YAP_PHASE5_GATE_BASE_URL must be the explicit loopback tunnel origin.");
  }
  if (
    parsed.origin !== baseUrl
    || parsed.protocol !== "http:"
    || parsed.hostname !== "127.0.0.1"
    || parsed.port !== "18765"
  ) {
    throw new Error(
      "YAP_PHASE5_GATE_BASE_URL must be exactly http://127.0.0.1:18765 for the explicit SSH forward.",
    );
  }
}

function requirePrivateEvidenceDirectory() {
  if (!path.isAbsolute(evidenceDirectory)) {
    throw new Error("YAP_PHASE5_GATE_EVIDENCE_DIR must be a new absolute private directory.");
  }
  const relative = path.relative(repoRoot, evidenceDirectory);
  if (relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative))) {
    throw new Error("Phase 5 gate evidence must stay outside the repository.");
  }
  const parent = path.dirname(evidenceDirectory);
  if (!existsSync(parent) || !statSync(parent).isDirectory() || lstatSync(parent).isSymbolicLink()) {
    throw new Error("The private Phase 5 evidence parent must be an existing real directory.");
  }
  const canonicalParent = realpathSync.native(parent);
  if (!sameWindowsPath(canonicalParent, parent)) {
    throw new Error("The private Phase 5 evidence parent must not redirect elsewhere.");
  }
  if (!worker) {
    mkdirSync(evidenceDirectory);
  }
  if (
    !existsSync(evidenceDirectory)
    || !statSync(evidenceDirectory).isDirectory()
    || lstatSync(evidenceDirectory).isSymbolicLink()
    || !sameWindowsPath(realpathSync.native(evidenceDirectory), evidenceDirectory)
  ) {
    throw new Error("The launcher-owned Phase 5 evidence directory is unavailable.");
  }
}

function stageLicensedFixture() {
  const runRoot = process.env.YAP_WDIO_RUN_ROOT;
  const appDataRoot = process.env.YAP_APP_DATA_DIR;
  if (!runRoot || !path.isAbsolute(runRoot) || !appDataRoot || !path.isAbsolute(appDataRoot)) {
    throw new Error("The Phase 5 gate requires the WDIO-owned private run roots.");
  }
  const lockPath = path.join(repoRoot, "server", "model-pools.lock.json");
  const lock = JSON.parse(readFileSync(lockPath, "utf8"));
  const source = path.join(repoRoot, ...lock.fixture.path.split("/"));
  const bytes = readFileSync(source);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (sha256 !== lock.fixture.sha256 || lock.fixture.license !== "CC-BY-4.0") {
    throw new Error("The Phase 5 gate fixture does not match its locked license and digest.");
  }

  const staged = path.join(runRoot, path.basename(source));
  if (!worker) {
    copyFileSync(source, staged, constants.COPYFILE_EXCL);
    writeFileSync(
      path.join(appDataRoot, "server-settings.json"),
      `${JSON.stringify({ schemaVersion: 1, enabled: true, baseUrl }, null, 2)}\n`,
      { encoding: "utf8", flag: "wx" },
    );
    writeFileSync(
      path.join(appDataRoot, "server-origin-approval.json"),
      `${JSON.stringify({ schemaVersion: 1, origin: baseUrl }, null, 2)}\n`,
      { encoding: "utf8", flag: "wx" },
    );
    writeFileSync(
      path.join(evidenceDirectory, "gate-context.json"),
      `${JSON.stringify({
        schemaVersion: 1,
        checkedHead,
        fixtureLicense: lock.fixture.license,
        fixtureSha256: sha256,
        serverOrigin: baseUrl,
        status: "started",
      }, null, 2)}\n`,
      { encoding: "utf8", flag: "wx" },
    );
  }
  if (!existsSync(staged) || !statSync(staged).isFile() || lstatSync(staged).isSymbolicLink()) {
    throw new Error("The launcher-owned Phase 5 fixture is unavailable.");
  }
  process.env.YAP_WDIO_PICKER_PATH = staged;
  process.env.YAP_PHASE5_GATE_FIXTURE_SHA256 = sha256;
  process.env.YAP_PHASE5_GATE_MODEL_ID = lock.pool.model.id;
  process.env.YAP_PHASE5_GATE_MODEL_REVISION = lock.pool.model.revision;
}

requireCheckedHead();
requireLoopbackGateOrigin();
requirePrivateEvidenceDirectory();
stageLicensedFixture();

const timeoutMs = resolvePhase5GateTimeout(process.env.YAP_PHASE5_GATE_TIMEOUT_MS);
process.env.YAP_PHASE5_GATE_TIMEOUT_MS = String(timeoutMs);

export const config = {
  ...baseConfig,
  bail: 1,
  mochaOpts: {
    ...baseConfig.mochaOpts,
    forbidOnly: true,
    forbidPending: true,
    timeout: timeoutMs,
  },
  outputDir: path.join(testsRoot, "results", "wdio-phase5-gate"),
  specs: [path.join(testsRoot, "wdio", "phase5-remote-stt.gate.spec.js")],
};
