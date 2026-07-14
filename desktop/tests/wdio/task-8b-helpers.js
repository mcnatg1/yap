import { randomUUID } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { rm as removeAsync } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const MICROPHONE_PERMISSION_DENIED_PREFIX = "Microphone permission denied:";

export async function registerTask8bLifecycleListeners(_tauri, options = {}) {
  const event = globalThis.__TAURI__?.event;
  if (!event?.listen) throw new Error("Tauri event API is unavailable in the current WebView.");
  const target = options.target ?? "overlay";
  if (target !== "main" && target !== "overlay") {
    throw new Error("Lifecycle listener target must be main or overlay.");
  }

  const state = {
    levels: [],
    saved: [],
    sessions: [],
    unlisteners: [],
  };
  globalThis.__yapTask8bLifecycle = state;
  state.cleanup = async () => {
    const pending = [...state.unlisteners];
    let cleaned = 0;
    const failures = [];
    for (const unlisten of pending) {
      try {
        await unlisten();
        const index = state.unlisteners.indexOf(unlisten);
        if (index >= 0) {
          state.unlisteners.splice(index, 1);
          cleaned += 1;
        }
      } catch (error) {
        failures.push(String(error));
      }
    }
    if (failures.length > 0) {
      throw new Error(`Lifecycle listener cleanup failed: ${failures.join("; ")}`);
    }
    return cleaned;
  };

  try {
    if (target === "overlay") {
      state.unlisteners.push(
        await event.listen("live-overlay-session", ({ payload }) => state.sessions.push(payload)),
      );
      state.unlisteners.push(
        await event.listen("live-level", ({ payload }) => state.levels.push(payload)),
      );
    } else {
      state.unlisteners.push(
        await event.listen("live-session-saved", ({ payload }) => state.saved.push(payload)),
      );
    }
    return state.unlisteners.length;
  } catch (registrationError) {
    try {
      await state.cleanup();
    } catch (cleanupError) {
      throw new Error(
        `${String(registrationError)}; partial listener cleanup also failed: ${String(cleanupError)}`,
      );
    }
    throw registrationError;
  }
}

export async function waitForTask8bSavedEvent(_tauri, options = {}) {
  const expectedCount = options.expectedCount ?? 1;
  const pollIntervalMs = options.pollIntervalMs ?? 25;
  const timeoutMs = options.timeoutMs ?? 5_000;
  if (!Number.isInteger(expectedCount) || expectedCount < 1) {
    throw new Error("Saved-event barrier expectedCount must be a positive integer.");
  }
  if (!Number.isFinite(pollIntervalMs) || pollIntervalMs <= 0) {
    throw new Error("Saved-event barrier pollIntervalMs must be positive.");
  }
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    throw new Error("Saved-event barrier timeoutMs must be positive.");
  }

  const deadline = Date.now() + timeoutMs;
  while (true) {
    const state = globalThis.__yapTask8bLifecycle;
    if (!state || !Array.isArray(state.saved)) {
      throw new Error("Task 8b lifecycle state is unavailable while waiting for a saved event.");
    }
    if (state.saved.length >= expectedCount) {
      return {
        levels: [...state.levels],
        saved: [...state.saved],
        sessions: [...state.sessions],
      };
    }

    const remainingMs = deadline - Date.now();
    if (remainingMs <= 0) {
      throw new Error(
        `Timed out waiting for ${expectedCount} saved event(s); received ${state.saved.length}.`,
      );
    }
    await new Promise((resolve) => {
      setTimeout(resolve, Math.min(pollIntervalMs, remainingMs));
    });
  }
}

const canonicalSessionName = /^live-s-[0-9a-f]{1,32}-[0-9a-f]{1,32}-[0-9a-f]{1,32}$/;
const expectedBundleSuffixes = [
  ".capture.json",
  ".commit.json",
  ".transcript.r1.json",
  ".txt",
  ".wav",
];

function stripExtendedWindowsPrefix(candidate) {
  if (/^\\\\\?\\UNC\\/i.test(candidate)) return `\\\\${candidate.slice(8)}`;
  if (/^\\\\\?\\/i.test(candidate)) return candidate.slice(4);
  return candidate;
}

function normalizeWindowsPath(candidate) {
  return path.win32.normalize(stripExtendedWindowsPrefix(candidate));
}

function sameWindowsPath(left, right) {
  return normalizeWindowsPath(left).toLocaleLowerCase("en-US")
    === normalizeWindowsPath(right).toLocaleLowerCase("en-US");
}

function requireAbsoluteWindowsPath(candidate, label) {
  if (typeof candidate !== "string" || !path.win32.isAbsolute(candidate)) {
    throw new Error(`${label} must be an absolute Windows path.`);
  }
  return normalizeWindowsPath(candidate);
}

function isNarrowPermissionDenial(error) {
  return typeof error === "string"
    && error.startsWith(MICROPHONE_PERMISSION_DENIED_PREFIX)
    && error.slice(MICROPHONE_PERMISSION_DENIED_PREFIX.length).trim().length > 0;
}

export function classifyNativeReadiness(environment) {
  const modelStatus = environment?.model?.status;
  const modelSkipStatuses = ["missing", "disabled", "corrupted"];
  if (modelStatus !== "ready" && !modelSkipStatuses.includes(modelStatus)) {
    throw new Error(`Unexpected Nemotron model status: ${modelStatus ?? "unknown"}`);
  }

  if (environment.deviceError) {
    if (isNarrowPermissionDenial(environment.deviceError)) {
      return {
        action: "skip",
        reason: `microphone permission was denied during enumeration: ${environment.deviceError}`,
      };
    }
    throw new Error(`Input device enumeration failed: ${environment.deviceError}`);
  }
  if (!Array.isArray(environment.devices)) {
    throw new Error("Input device enumeration failed: native command returned no device list");
  }
  if (environment.devices.length === 0) {
    return { action: "skip", reason: "no input device was enumerated" };
  }

  const preflight = environment.preflight;
  if (!preflight || typeof preflight.status !== "string") {
    throw new Error("Unexpected microphone preflight failure: native command returned no status");
  }
  if (preflight.status === "blocked") {
    const error = preflight.error;
    if (isNarrowPermissionDenial(error)) {
      return { action: "skip", reason: `microphone preflight permission was denied: ${error}` };
    }
    throw new Error(`Unexpected microphone preflight failure: ${error || "unknown error"}`);
  }
  if (preflight.status !== "idle") {
    throw new Error(`Unexpected microphone preflight status: ${preflight.status}`);
  }
  if (modelSkipStatuses.includes(modelStatus)) {
    return { action: "skip", reason: `Nemotron model is ${modelStatus}` };
  }
  return { action: "run" };
}

function listRelativeEntries(root, relative = "") {
  const current = relative ? path.join(root, relative) : root;
  const entries = [];
  for (const entry of readdirSync(current, { withFileTypes: true })) {
    const next = relative ? path.join(relative, entry.name) : entry.name;
    entries.push(next);
    if (entry.isDirectory() && !entry.isSymbolicLink()) {
      entries.push(...listRelativeEntries(root, next));
    }
  }
  return entries;
}

export function listRecordingArtifacts(recordingRoot) {
  if (!existsSync(recordingRoot)) return [];
  return listRelativeEntries(recordingRoot).sort((left, right) => left.localeCompare(right));
}

export function assertRecordingRootEmpty(recordingRoot) {
  const artifacts = listRecordingArtifacts(recordingRoot);
  if (artifacts.length > 0) {
    throw new Error(`Isolated recording root is not empty: ${artifacts.join(", ")}`);
  }
}

function assertCanonicalFile(candidate, expectedName, label, root, canonicalRoot, canonicalize) {
  const normalized = requireAbsoluteWindowsPath(candidate, label);
  const expected = path.win32.join(root, expectedName);
  if (!sameWindowsPath(normalized, expected)) {
    throw new Error(`${label} violates the canonical name/session relationship or isolated recording root.`);
  }
  if (!existsSync(candidate)) throw new Error(`${label} does not exist: ${candidate}`);
  const linkMetadata = lstatSync(candidate);
  if (linkMetadata.isSymbolicLink()) throw new Error(`${label} must not be a symbolic link.`);
  if (!statSync(candidate).isFile()) throw new Error(`${label} must be a file.`);

  const canonical = requireAbsoluteWindowsPath(canonicalize(candidate), `${label} canonical path`);
  if (!sameWindowsPath(path.win32.dirname(canonical), canonicalRoot)) {
    throw new Error(`${label} canonical parent is not the isolated recording root.`);
  }
  if (!sameWindowsPath(path.win32.basename(canonical), expectedName)) {
    throw new Error(`${label} canonical filename does not match the session.`);
  }
}

export function assertOwnedSavedSession(saved, recordingRoot, options = {}) {
  const canonicalize = options.canonicalize ?? realpathSync.native;
  const nowMs = options.nowMs ?? Date.now();
  const runStartedAtMs = options.runStartedAtMs;
  const normalizedRoot = requireAbsoluteWindowsPath(recordingRoot, "Isolated recording root");
  if (!existsSync(recordingRoot) || !statSync(recordingRoot).isDirectory()) {
    throw new Error("Isolated recording root must exist and be a directory.");
  }
  const canonicalRoot = requireAbsoluteWindowsPath(
    canonicalize(recordingRoot),
    "Canonical isolated recording root",
  );
  if (!sameWindowsPath(canonicalRoot, normalizedRoot)) {
    throw new Error("Isolated recording root resolves outside itself.");
  }
  if (!saved || typeof saved !== "object" || !canonicalSessionName.test(saved.name ?? "")) {
    throw new Error("Saved event has no canonical live-s-* name/session relationship.");
  }
  const expectedSessionId = saved.name.slice("live-".length);
  if (saved.sessionId !== expectedSessionId) {
    throw new Error("Saved event opaque session ID does not match its canonical artifacts.");
  }
  if (!Number.isFinite(runStartedAtMs)
    || !Number.isFinite(saved.createdAtMs)
    || saved.createdAtMs < runStartedAtMs
    || saved.createdAtMs > nowMs + 5_000) {
    throw new Error("Saved event does not belong to the current test run.");
  }

  const artifactNames = expectedBundleSuffixes.map((suffix) => `${saved.name}${suffix}`);
  const eventPaths = new Map([
    [`${saved.name}.commit.json`, [saved.captureCommitPath, "capture commit path"]],
    [`${saved.name}.txt`, [saved.outputPath, "transcript output path"]],
    [`${saved.name}.wav`, [saved.sourcePath, "recording source path"]],
  ]);
  for (const [artifactName, [eventPath, label]] of eventPaths) {
    assertCanonicalFile(
      eventPath,
      artifactName,
      label,
      normalizedRoot,
      canonicalRoot,
      canonicalize,
    );
  }

  const actualNames = readdirSync(recordingRoot, { withFileTypes: true })
    .map((entry) => entry.name)
    .sort((left, right) => left.localeCompare(right));
  if (actualNames.length !== artifactNames.length
    || actualNames.some((name, index) => name !== artifactNames[index])) {
    throw new Error(
      `Isolated recording root must contain exactly the expected artifacts for ${saved.name}; found ${actualNames.join(", ") || "none"}.`,
    );
  }

  for (const artifactName of artifactNames) {
    if (eventPaths.has(artifactName)) continue;
    assertCanonicalFile(
      path.win32.join(normalizedRoot, artifactName),
      artifactName,
      `session artifact ${artifactName}`,
      normalizedRoot,
      canonicalRoot,
      canonicalize,
    );
  }
  if (statSync(saved.sourcePath).size <= 44) {
    throw new Error("Owned recording WAV contains no PCM payload.");
  }

  return {
    artifactNames,
    sessionId: saved.sessionId,
  };
}

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
