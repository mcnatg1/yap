import {
  existsSync,
  lstatSync,
  readdirSync,
  realpathSync,
  statSync,
} from "node:fs";
import path from "node:path";

import {
  requireAbsoluteWindowsPath,
  sameWindowsPath,
} from "./task-8b-paths.js";


const canonicalSessionName = /^live-s-[0-9a-f]{1,32}-[0-9a-f]{1,32}-[0-9a-f]{1,32}$/;
const expectedBundleSuffixes = [
  ".capture.json",
  ".commit.json",
  ".transcript.r1.json",
  ".txt",
  ".wav",
];


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
