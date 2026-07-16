import { randomUUID } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { rm as removeAsync } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { listRecordingArtifacts } from "./task-8b-artifacts.js";
import {
  requireAbsoluteWindowsPath,
  sameWindowsPath,
} from "./task-8b-paths.js";


function generatedToken() {
  return `${new Date().toISOString().replace(/[^0-9]/g, "")}-${process.pid}-${randomUUID()}`;
}

const helperDirectory = path.dirname(fileURLToPath(import.meta.url));
const fixedWdioTempRoot = path.resolve(helperDirectory, "..", "results", "temp", "wdio");
const isolationTokenPattern = /^[a-z0-9](?:[a-z0-9-]{0,126}[a-z0-9])?$/;
const retryableRemovalCodes = new Set(["EBUSY", "EMFILE", "ENFILE", "ENOTEMPTY", "EPERM"]);
const runOwnerMarkerName = ".yap-wdio-run-owner";

function expectedIsolationPaths(token) {
  const runRoot = path.join(fixedWdioTempRoot, `run-${token}`);
  return {
    appDataRoot: path.join(runRoot, "app-data"),
    modelsRoot: path.join(runRoot, "models"),
    recordingRoot: path.join(runRoot, "live-recordings"),
    runRoot,
    tempRoot: fixedWdioTempRoot,
    webviewRoot: path.join(runRoot, "webview2"),
  };
}

function assertUnredirectedDirectory(candidate, label) {
  if (!existsSync(candidate)) return false;
  const metadata = lstatSync(candidate);
  if (metadata.isSymbolicLink()) {
    throw new Error(`${label} must not be a link or reparse redirect.`);
  }
  if (!metadata.isDirectory()) {
    throw new Error(`${label} must be a directory.`);
  }
  const canonical = requireAbsoluteWindowsPath(realpathSync.native(candidate), `Canonical ${label}`);
  if (!sameWindowsPath(canonical, candidate)) {
    throw new Error(`${label} resolves through a canonical redirect.`);
  }
  return true;
}

function assertSafeIsolationRoot(isolation) {
  if (!isolation || typeof isolation !== "object") {
    throw new Error("WDIO isolation proof is required for cleanup.");
  }
  const { token } = isolation;
  if (typeof token !== "string" || !isolationTokenPattern.test(token)) {
    throw new Error("WDIO isolation token contains unsupported characters.");
  }

  const expected = expectedIsolationPaths(token);
  const tempRoot = requireAbsoluteWindowsPath(isolation.tempRoot, "WDIO temp root");
  if (!sameWindowsPath(tempRoot, expected.tempRoot)) {
    throw new Error("WDIO temp root must equal the fixed WDIO temp root.");
  }
  const runRoot = requireAbsoluteWindowsPath(isolation.runRoot, "WDIO run root");
  if (!sameWindowsPath(runRoot, expected.runRoot)) {
    throw new Error("WDIO token and run root do not have the required exact relationship.");
  }
  const recordingRoot = requireAbsoluteWindowsPath(
    isolation.recordingRoot,
    "WDIO private recording root",
  );
  if (!sameWindowsPath(recordingRoot, expected.recordingRoot)) {
    throw new Error("WDIO recording root is not the exact child of the token-derived run root.");
  }
  const appDataRoot = requireAbsoluteWindowsPath(isolation.appDataRoot, "WDIO app-data root");
  if (!sameWindowsPath(appDataRoot, expected.appDataRoot)) {
    throw new Error("WDIO app-data root is not the exact child of the token-derived run root.");
  }
  const modelsRoot = requireAbsoluteWindowsPath(isolation.modelsRoot, "WDIO models root");
  if (!sameWindowsPath(modelsRoot, expected.modelsRoot)) {
    throw new Error("WDIO models root is not the exact child of the token-derived run root.");
  }
  const webviewRoot = requireAbsoluteWindowsPath(isolation.webviewRoot, "WDIO WebView root");
  if (!sameWindowsPath(webviewRoot, expected.webviewRoot)) {
    throw new Error("WDIO WebView root is not the exact child of the token-derived run root.");
  }

  const tempExists = assertUnredirectedDirectory(expected.tempRoot, "Fixed WDIO temp root");
  const runExists = assertUnredirectedDirectory(expected.runRoot, "WDIO run root");
  if (runExists && !tempExists) {
    throw new Error("WDIO run root exists without the fixed WDIO temp root.");
  }
  if (runExists) {
    const ownerMarker = path.join(expected.runRoot, runOwnerMarkerName);
    if (!existsSync(ownerMarker)) {
      throw new Error("WDIO run root is missing its exclusive ownership marker.");
    }
    const markerMetadata = lstatSync(ownerMarker);
    if (markerMetadata.isSymbolicLink() || !markerMetadata.isFile()) {
      throw new Error("WDIO run-root ownership marker must be an unredirected regular file.");
    }
    if (readFileSync(ownerMarker, "utf8") !== `${token}\n`) {
      throw new Error("WDIO run-root ownership marker does not match this run token.");
    }
  }
  assertUnredirectedDirectory(expected.recordingRoot, "WDIO recording root");
  assertUnredirectedDirectory(expected.appDataRoot, "WDIO app-data root");
  assertUnredirectedDirectory(expected.modelsRoot, "WDIO models root");
  assertUnredirectedDirectory(expected.webviewRoot, "WDIO WebView root");
  return expected;
}

export function createWdioRunIsolation(env = process.env, options = {}) {
  const token = options.token ?? env.YAP_WDIO_RUN_TOKEN ?? generatedToken();
  if (typeof token !== "string" || !isolationTokenPattern.test(token)) {
    throw new Error("WDIO isolation token contains unsupported characters.");
  }
  const isolation = { ...expectedIsolationPaths(token), token };

  mkdirSync(isolation.tempRoot, { recursive: true });
  assertUnredirectedDirectory(isolation.tempRoot, "Fixed WDIO temp root");
  try {
    mkdirSync(isolation.runRoot);
  } catch (error) {
    if (error?.code === "EEXIST") {
      throw new Error(`WDIO run root already exists and cannot be inherited or reclaimed: ${isolation.runRoot}`);
    }
    throw error;
  }
  assertUnredirectedDirectory(isolation.runRoot, "WDIO run root");
  writeFileSync(path.join(isolation.runRoot, runOwnerMarkerName), `${token}\n`, {
    encoding: "utf8",
    flag: "wx",
  });
  if (!existsSync(isolation.appDataRoot)) mkdirSync(isolation.appDataRoot);
  if (!existsSync(isolation.modelsRoot)) mkdirSync(isolation.modelsRoot);
  if (!existsSync(isolation.recordingRoot)) mkdirSync(isolation.recordingRoot);
  if (!existsSync(isolation.webviewRoot)) mkdirSync(isolation.webviewRoot);
  assertSafeIsolationRoot(isolation);

  env.YAP_WDIO_RUN_TOKEN = token;
  env.YAP_WDIO_RUN_ROOT = isolation.runRoot;
  env.YAP_APP_DATA_DIR = isolation.appDataRoot;
  env.YAP_MODELS_DIR = isolation.modelsRoot;
  env.YAP_LIVE_RECORDINGS_DIR = isolation.recordingRoot;
  env.WEBVIEW2_USER_DATA_FOLDER = isolation.webviewRoot;
  return isolation;
}

export function attachWdioRunIsolation(env = process.env) {
  const token = env.YAP_WDIO_RUN_TOKEN;
  if (typeof token !== "string" || !isolationTokenPattern.test(token)) {
    throw new Error("Inherited WDIO isolation token is missing or invalid.");
  }

  const isolation = { ...expectedIsolationPaths(token), token };
  const inheritedPaths = [
    ["YAP_WDIO_RUN_ROOT", "runRoot", "run root"],
    ["YAP_APP_DATA_DIR", "appDataRoot", "app-data root"],
    ["YAP_MODELS_DIR", "modelsRoot", "models root"],
    ["YAP_LIVE_RECORDINGS_DIR", "recordingRoot", "recording root"],
    ["WEBVIEW2_USER_DATA_FOLDER", "webviewRoot", "WebView root"],
  ];
  for (const [variable, key, label] of inheritedPaths) {
    const inherited = requireAbsoluteWindowsPath(env[variable], `Inherited WDIO ${label}`);
    if (!sameWindowsPath(inherited, isolation[key])) {
      throw new Error(`Inherited WDIO ${label} does not match the launcher-owned isolation.`);
    }
  }
  assertSafeIsolationRoot(isolation);
  return isolation;
}

async function removeOwnedRunRoot(isolation, removeDirectory) {
  const { runRoot } = assertSafeIsolationRoot(isolation);
  const children = readdirSync(runRoot)
    .filter((name) => name !== runOwnerMarkerName);

  for (const child of children) {
    await removeDirectory(path.join(runRoot, child), { force: true, recursive: true });
  }

  // Keep the ownership proof until all potentially locked children are gone.
  await removeDirectory(runRoot, { force: true, recursive: true });
}

async function safeRemovePrivateDirectory(isolation, targetName, options = {}) {
  const maxAttempts = options.maxAttempts ?? 150;
  const retryDelayMs = options.retryDelayMs ?? 100;
  const removeDirectory = options.removeDirectory ?? removeAsync;
  if (!Number.isInteger(maxAttempts) || maxAttempts < 1) {
    throw new Error("Private cleanup maxAttempts must be a positive integer.");
  }
  if (!Number.isFinite(retryDelayMs) || retryDelayMs < 0) {
    throw new Error("Private cleanup retryDelayMs must not be negative.");
  }
  if (targetName !== "recordingRoot" && targetName !== "runRoot") {
    throw new Error("Private cleanup target is not allowed.");
  }

  let lastError;
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const target = assertSafeIsolationRoot(isolation)[targetName];
    if (!existsSync(target)) return;

    try {
      if (targetName === "runRoot") {
        await removeOwnedRunRoot(isolation, removeDirectory);
      } else {
        await removeDirectory(target, { force: true, recursive: true });
      }
      if (!existsSync(target)) return;
      lastError = new Error("recursive removal returned while the target still existed");
    } catch (error) {
      if (!retryableRemovalCodes.has(error?.code)) throw error;
      lastError = error;
    }

    if (attempt === maxAttempts) break;
    if (options.onRetry) await options.onRetry({ attempt, target });
    if (retryDelayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, retryDelayMs));
    }
  }

  const target = expectedIsolationPaths(isolation.token)[targetName];
  throw new Error(
    `Private WDIO ${targetName} remained locked after ${maxAttempts} attempts: ${target}; ${String(lastError)}`,
  );
}

export async function resetPrivateRecordingRoot(isolation, options = {}) {
  const { recordingRoot, runRoot } = assertSafeIsolationRoot(isolation);
  if (!existsSync(runRoot)) {
    throw new Error("Cannot reset a recording root after its private WDIO run root was removed.");
  }
  const artifacts = listRecordingArtifacts(recordingRoot);
  await safeRemovePrivateDirectory(isolation, "recordingRoot", options);

  const revalidated = assertSafeIsolationRoot(isolation);
  if (!existsSync(revalidated.runRoot)) {
    throw new Error("Private WDIO run root disappeared before recording-root recreation.");
  }
  if (!existsSync(revalidated.recordingRoot)) mkdirSync(revalidated.recordingRoot);
  assertSafeIsolationRoot(isolation);
  return artifacts;
}

export async function removePrivateRunRootWhenReleased(isolation, options = {}) {
  await safeRemovePrivateDirectory(isolation, "runRoot", options);
}
