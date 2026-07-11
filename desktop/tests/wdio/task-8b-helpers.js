import { randomUUID } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
} from "node:fs";
import { rm as removeAsync } from "node:fs/promises";
import path from "node:path";

export const MICROPHONE_PERMISSION_DENIED_PREFIX = "Microphone permission denied:";

export async function registerTask8bLifecycleListeners() {
  const event = globalThis.__TAURI__?.event;
  if (!event?.listen) throw new Error("Tauri event API is unavailable in the overlay WebView.");

  const state = {
    levels: [],
    saved: [],
    sessions: [],
    unlisteners: [],
  };
  globalThis.__yapTask8bLifecycle = state;
  state.cleanup = async () => {
    const unlisteners = state.unlisteners.splice(0);
    const failures = [];
    for (const unlisten of unlisteners) {
      try {
        await unlisten();
      } catch (error) {
        failures.push(String(error));
      }
    }
    if (failures.length > 0) {
      throw new Error(`Lifecycle listener cleanup failed: ${failures.join("; ")}`);
    }
    return unlisteners.length;
  };

  try {
    state.unlisteners.push(
      await event.listen("live-session", ({ payload }) => state.sessions.push(payload)),
    );
    state.unlisteners.push(
      await event.listen("live-level", ({ payload }) => state.levels.push(payload)),
    );
    state.unlisteners.push(
      await event.listen("live-session-saved", ({ payload }) => state.saved.push(payload)),
    );
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
    sessionId: saved.name.slice("live-".length),
  };
}

function generatedToken() {
  return `${new Date().toISOString().replace(/[^0-9]/g, "")}-${process.pid}-${randomUUID()}`;
}

function assertSafeIsolationRoot(isolation) {
  const tempRoot = requireAbsoluteWindowsPath(isolation.tempRoot, "WDIO temp root");
  const runRoot = requireAbsoluteWindowsPath(isolation.runRoot, "WDIO run root");
  if (!sameWindowsPath(path.win32.dirname(runRoot), tempRoot)
    || !path.win32.basename(runRoot).startsWith("run-")) {
    throw new Error("Refusing cleanup outside the generated WDIO temp root.");
  }
  if (existsSync(tempRoot)) {
    const canonicalTemp = requireAbsoluteWindowsPath(realpathSync.native(tempRoot), "Canonical WDIO temp root");
    if (!sameWindowsPath(canonicalTemp, tempRoot)) {
      throw new Error("Refusing cleanup through a redirected WDIO temp root.");
    }
  }
  if (existsSync(runRoot)) {
    const canonicalRun = requireAbsoluteWindowsPath(realpathSync.native(runRoot), "Canonical WDIO run root");
    if (!sameWindowsPath(canonicalRun, runRoot)
      || !sameWindowsPath(path.win32.dirname(canonicalRun), tempRoot)) {
      throw new Error("Refusing cleanup through a redirected WDIO run root.");
    }
  }
  return { runRoot, tempRoot };
}

export function createWdioRunIsolation(testsRoot, env = process.env, options = {}) {
  const tempRoot = path.resolve(testsRoot, "results", "temp", "wdio");
  const token = options.token ?? env.YAP_WDIO_RUN_TOKEN ?? generatedToken();
  if (!/^[a-z0-9][a-z0-9-]{0,127}$/i.test(token)) {
    throw new Error("WDIO isolation token contains unsupported characters.");
  }
  const runRoot = path.resolve(tempRoot, `run-${token}`);
  const recordingRoot = path.join(runRoot, "live-recordings");
  const webviewRoot = path.join(runRoot, "webview2");
  mkdirSync(recordingRoot, { recursive: true });
  mkdirSync(webviewRoot, { recursive: true });

  env.YAP_WDIO_RUN_TOKEN = token;
  env.YAP_WDIO_RUN_ROOT = runRoot;
  env.YAP_LIVE_RECORDINGS_DIR = recordingRoot;
  env.WEBVIEW2_USER_DATA_FOLDER = webviewRoot;

  const isolation = { recordingRoot, runRoot, tempRoot, token, webviewRoot };
  assertSafeIsolationRoot(isolation);
  return isolation;
}

export function resetPrivateRecordingRoot(isolation) {
  const { runRoot } = assertSafeIsolationRoot(isolation);
  const recordingRoot = requireAbsoluteWindowsPath(
    isolation.recordingRoot,
    "WDIO private recording root",
  );
  if (!sameWindowsPath(recordingRoot, path.win32.join(runRoot, "live-recordings"))) {
    throw new Error("Refusing to reset a recording root outside the generated WDIO run root.");
  }
  const artifacts = listRecordingArtifacts(recordingRoot);
  if (existsSync(recordingRoot)) {
    rmSync(recordingRoot, {
      force: true,
      maxRetries: 100,
      recursive: true,
      retryDelay: 100,
    });
  }
  mkdirSync(recordingRoot, { recursive: true });
  return artifacts;
}

export function removePrivateRunRoot(isolation) {
  const { runRoot } = assertSafeIsolationRoot(isolation);
  if (existsSync(runRoot)) {
    rmSync(runRoot, {
      force: true,
      maxRetries: 100,
      recursive: true,
      retryDelay: 100,
    });
  }
}

export async function removePrivateRunRootWhenReleased(isolation) {
  const { runRoot } = assertSafeIsolationRoot(isolation);
  const retryableCodes = new Set(["EBUSY", "EMFILE", "ENFILE", "ENOTEMPTY", "EPERM"]);
  let lastError;
  for (let attempt = 0; attempt < 150; attempt += 1) {
    try {
      await removeAsync(runRoot, { force: true, recursive: true });
      if (!existsSync(runRoot)) return;
    } catch (error) {
      if (!retryableCodes.has(error?.code)) throw error;
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(
    `Private WDIO run root remained locked after service shutdown: ${runRoot}; ${String(lastError)}`,
  );
}
