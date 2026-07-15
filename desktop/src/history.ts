export type TranscriptHistoryEntry = {
  captureCommitPath?: string;
  name: string;
  origin?: "live" | "remote";
  sourcePath: string;
  outputPath: string;
  sessionId?: string;
  createdAt: string;
  warning?: string;
  recoveryState?: "recoverable" | "recovered";
};

export type SavedTranscriptSession = {
  captureCommitPath?: string | null;
  createdAtMs: number;
  name: string;
  origin?: "live" | "remote";
  sourcePath: string;
  outputPath: string;
  sessionId: string;
  warning?: string | null;
  recoveryState?: "recoverable" | "recovered" | null;
};

const transcriptHistoryKey = "yap.transcriptHistory.v1";
const hiddenTranscriptHistoryKey = "yap.hiddenTranscriptHistory.v1";
export const maxTranscriptHistoryEntries = 500;

export type HistoryStorage = Pick<Storage, "getItem" | "setItem">;

export type OwnedLiveTranscriptPathResolution = {
  requestedPath: string;
  canonicalPath?: string | null;
  missing: boolean;
};

export type SavedLiveSessionActionIdentity = {
  expectedCaptureCommitPath: string;
  expectedOutputPath: string;
  sessionId: string;
};

export type RecoverableLiveSessionActionIdentity = {
  expectedArtifactPath: string;
  sessionId: string;
};

const hiddenPruneBatchSize = 200;
const trustedNativeHistoryIdentities = new Map<
  string,
  { createdAtMs: number; outputIdentity: string; sessionId: string }
>();

function isHistoryEntry(value: unknown): value is TranscriptHistoryEntry {
  if (!value || typeof value !== "object") return false;
  const entry = value as Record<string, unknown>;
  return (
    typeof entry.name === "string" &&
    typeof entry.sourcePath === "string" &&
    typeof entry.outputPath === "string" &&
    typeof entry.createdAt === "string" &&
    (entry.origin === undefined || entry.origin === "live" || entry.origin === "remote") &&
    (entry.sessionId === undefined || typeof entry.sessionId === "string") &&
    (entry.warning === undefined || typeof entry.warning === "string") &&
    (entry.captureCommitPath === undefined || typeof entry.captureCommitPath === "string") &&
    (entry.recoveryState === undefined || entry.recoveryState === "recoverable" || entry.recoveryState === "recovered")
  );
}

function withoutNativeHistoryOrigin(entry: TranscriptHistoryEntry): TranscriptHistoryEntry {
  const { origin: _origin, ...storedEntry } = entry;
  return storedEntry;
}

export function transcriptPathIdentity(path: string) {
  const isWindowsPath = /^[a-z]:[\\/]/i.test(path) || /^(?:\\\\|\/\/)/.test(path);
  if (!isWindowsPath) return path;

  let normalized = path
    .replace(/^\\\\\?\\UNC\\/i, "\\\\")
    .replace(/^\\\\\?\\/i, "")
    .replace(/\//g, "\\");
  const unc = normalized.startsWith("\\\\");
  const segments = normalized.split("\\").filter(Boolean);
  const resolved: string[] = [];
  const rootDepth = unc ? 2 : 1;
  for (const segment of segments) {
    if (segment === ".") continue;
    if (segment === "..") {
      if (resolved.length > rootDepth) resolved.pop();
      continue;
    }
    resolved.push(segment);
  }
  normalized = `${unc ? "\\\\" : ""}${resolved.join("\\")}`;
  return normalized.toLowerCase();
}

function normalizeTranscriptHistory(value: unknown) {
  if (!Array.isArray(value)) return [];

  const seen = new Set<string>();
  return value
    .filter(isHistoryEntry)
    .filter((entry) => {
      const identity = transcriptPathIdentity(entry.outputPath);
      if (seen.has(identity)) return false;
      seen.add(identity);
      return true;
    })
    .sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))
    .slice(0, maxTranscriptHistoryEntries);
}

export function normalizeHiddenTranscriptHistory(value: unknown) {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  return value.filter((item): item is string => {
    if (typeof item !== "string") return false;
    const identity = transcriptPathIdentity(item);
    if (seen.has(identity)) return false;
    seen.add(identity);
    return true;
  });
}

export function readTranscriptHistory(storage: HistoryStorage | undefined = globalThis.localStorage) {
  if (!storage) return [];

  try {
    return normalizeTranscriptHistory(JSON.parse(storage.getItem(transcriptHistoryKey) ?? "[]"))
      .map(withoutNativeHistoryOrigin);
  } catch {
    return [];
  }
}

export function readHiddenTranscriptHistory(storage: HistoryStorage | undefined = globalThis.localStorage) {
  if (!storage) return [];

  try {
    return normalizeHiddenTranscriptHistory(JSON.parse(storage.getItem(hiddenTranscriptHistoryKey) ?? "[]"));
  } catch {
    return [];
  }
}

export function filterHiddenTranscriptHistory(entries: TranscriptHistoryEntry[], outputPaths: string[]) {
  const hidden = new Set(
    normalizeHiddenTranscriptHistory(outputPaths).map(transcriptPathIdentity),
  );
  if (!hidden.size) return entries;
  return entries.filter((entry) => !hidden.has(transcriptPathIdentity(entry.outputPath)));
}

function historyPathBasename(path: string) {
  return path.replace(/\\/g, "/").split("/").pop()?.toLowerCase() ?? "";
}

function isPreReleaseLiveHistoryEntry(entry: TranscriptHistoryEntry) {
  const name = entry.name.toLowerCase();
  return /^live-\d+(?:-\d+)?$/.test(name)
    && historyPathBasename(entry.outputPath) === `${name}.txt`
    && historyPathBasename(entry.sourcePath) === `${name}.wav`;
}

function isCanonicalYapLiveHistoryEntry(entry: TranscriptHistoryEntry) {
  return Boolean(entry.captureCommitPath || entry.recoveryState);
}

function nativeHistoryProvenanceKey(entry: TranscriptHistoryEntry) {
  if (entry.origin !== "live") return undefined;
  const sessionId = validHistorySessionId(entry);
  if (!sessionId || !isCanonicalYapLiveHistoryEntry(entry)) return undefined;
  return JSON.stringify([
    sessionId,
    transcriptPathIdentity(entry.sourcePath),
    transcriptPathIdentity(entry.outputPath),
    entry.captureCommitPath ? transcriptPathIdentity(entry.captureCommitPath) : "",
    entry.recoveryState ?? "",
  ]);
}

function trustNativeTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  const identity = nativeHistoryProvenanceKey(entry);
  const sessionId = validHistorySessionId(entry);
  if (!identity || !sessionId) return entry;

  const outputIdentity = transcriptPathIdentity(entry.outputPath);
  for (const [trustedIdentity, trusted] of trustedNativeHistoryIdentities) {
    if (trusted.outputIdentity === outputIdentity || trusted.sessionId === sessionId) {
      trustedNativeHistoryIdentities.delete(trustedIdentity);
    }
  }
  const parsedCreatedAt = Date.parse(entry.createdAt);
  trustedNativeHistoryIdentities.set(identity, {
    createdAtMs: Number.isFinite(parsedCreatedAt) ? parsedCreatedAt : 0,
    outputIdentity,
    sessionId,
  });
  while (trustedNativeHistoryIdentities.size > maxTranscriptHistoryEntries) {
    let oldestIdentity: string | undefined;
    let oldestCreatedAt = Number.POSITIVE_INFINITY;
    for (const [trustedIdentity, trusted] of trustedNativeHistoryIdentities) {
      if (trusted.createdAtMs < oldestCreatedAt) {
        oldestCreatedAt = trusted.createdAtMs;
        oldestIdentity = trustedIdentity;
      }
    }
    if (oldestIdentity === undefined) break;
    trustedNativeHistoryIdentities.delete(oldestIdentity);
  }
  return entry;
}

function replaceNativeTranscriptHistoryTrust(entries: TranscriptHistoryEntry[]) {
  trustedNativeHistoryIdentities.clear();
  for (const entry of normalizeTranscriptHistory(entries)) {
    trustNativeTranscriptHistoryEntry(entry);
  }
}

function revokeNativeTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  const identity = nativeHistoryProvenanceKey(entry);
  if (identity) trustedNativeHistoryIdentities.delete(identity);
}

export function readVisibleTranscriptHistory(storage: HistoryStorage | undefined = globalThis.localStorage) {
  if (!storage) return [];
  const history = readTranscriptHistory(storage);
  return filterHiddenTranscriptHistory(
    history.filter(
      (entry) => !isPreReleaseLiveHistoryEntry(entry),
    ),
    readHiddenTranscriptHistory(storage),
  );
}

export function writeTranscriptHistory(entries: TranscriptHistoryEntry[], storage: HistoryStorage = globalThis.localStorage) {
  const legacyEntries = entries
    .filter((entry) => entry.origin === undefined)
    .map(withoutNativeHistoryOrigin);
  storage.setItem(transcriptHistoryKey, JSON.stringify(normalizeTranscriptHistory(legacyEntries)));
}

export function writeHiddenTranscriptHistory(outputPaths: string[], storage: HistoryStorage = globalThis.localStorage) {
  storage.setItem(hiddenTranscriptHistoryKey, JSON.stringify(normalizeHiddenTranscriptHistory(outputPaths)));
}

export async function pruneMissingHiddenTranscriptHistory(
  authorize: (outputPaths: string[]) => Promise<OwnedLiveTranscriptPathResolution[]>,
  storage: HistoryStorage | undefined = globalThis.localStorage,
) {
  if (!storage) return [];
  const initial = readHiddenTranscriptHistory(storage);
  if (!initial.length) return initial;

  const confirmedMissing = new Set<string>();
  const canonicalRewrites = new Map<string, string>();
  const staleAliasHistory = new Set<string>();
  for (let offset = 0; offset < initial.length; offset += hiddenPruneBatchSize) {
    const batch = initial.slice(offset, offset + hiddenPruneBatchSize);
    const requested = new Set(batch.map(transcriptPathIdentity));
    const resolutions = await authorize(batch);
    for (const resolution of resolutions) {
      const requestedIdentity = transcriptPathIdentity(resolution.requestedPath);
      if (!requested.has(requestedIdentity)) continue;
      if (resolution.missing) {
        confirmedMissing.add(requestedIdentity);
        if (resolution.canonicalPath) {
          confirmedMissing.add(transcriptPathIdentity(resolution.canonicalPath));
        }
        continue;
      }
      if (!resolution.canonicalPath) continue;
      const canonicalIdentity = transcriptPathIdentity(resolution.canonicalPath);
      if (resolution.canonicalPath !== resolution.requestedPath) {
        canonicalRewrites.set(requestedIdentity, resolution.canonicalPath);
        if (canonicalIdentity !== requestedIdentity) {
          staleAliasHistory.add(requestedIdentity);
        }
      }
    }
  }
  if (!confirmedMissing.size && !canonicalRewrites.size) {
    return readHiddenTranscriptHistory(storage);
  }

  const currentBeforeCleanup = readHiddenTranscriptHistory(storage);
  const protectedCanonicalPaths = normalizeHiddenTranscriptHistory([
    ...currentBeforeCleanup,
    ...canonicalRewrites.values(),
  ]);
  if (JSON.stringify(protectedCanonicalPaths) !== JSON.stringify(currentBeforeCleanup)) {
    writeHiddenTranscriptHistory(protectedCanonicalPaths, storage);
  }

  const history = readTranscriptHistory(storage);
  const nextHistory = history.filter(
    (entry) => {
      const identity = transcriptPathIdentity(entry.outputPath);
      return !confirmedMissing.has(identity) && !staleAliasHistory.has(identity);
    },
  );
  if (nextHistory.length !== history.length) {
    writeTranscriptHistory(nextHistory, storage);
  }

  const current = readHiddenTranscriptHistory(storage);
  const next = normalizeHiddenTranscriptHistory(current.flatMap((outputPath) => {
    const identity = transcriptPathIdentity(outputPath);
    if (confirmedMissing.has(identity)) return [];
    return [canonicalRewrites.get(identity) ?? outputPath];
  }));
  if (JSON.stringify(next) !== JSON.stringify(current)) {
    writeHiddenTranscriptHistory(next, storage);
  }
  return next;
}

export function recordTranscriptHistory(entries: TranscriptHistoryEntry[], entry: TranscriptHistoryEntry) {
  if (isPreReleaseLiveHistoryEntry(entry) && !isCanonicalYapLiveHistoryEntry(entry)) return entries;
  const identity = transcriptPathIdentity(entry.outputPath);
  return normalizeTranscriptHistory([
    entry,
    ...entries.filter((item) => transcriptPathIdentity(item.outputPath) !== identity),
  ]);
}

export function recordVisibleTranscriptHistoryEntries(
  current: TranscriptHistoryEntry[],
  entries: TranscriptHistoryEntry[],
  hiddenOutputPaths: string[],
) {
  const hidden = new Set(
    normalizeHiddenTranscriptHistory(hiddenOutputPaths).map(transcriptPathIdentity),
  );
  const visibleEntries = entries.filter(
    (entry) => !hidden.has(transcriptPathIdentity(entry.outputPath)),
  );
  if (!visibleEntries.length) return current;

  const visibleHistory = filterHiddenTranscriptHistory(current, hiddenOutputPaths);
  return visibleEntries.reduce(recordTranscriptHistory, visibleHistory);
}

export function isNativeLiveTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  const identity = nativeHistoryProvenanceKey(entry);
  return identity !== undefined && trustedNativeHistoryIdentities.has(identity);
}

export function isUntrustedNativeLiveTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  return isCanonicalYapLiveHistoryEntry(entry) && !isNativeLiveTranscriptHistoryEntry(entry);
}

export function isRecoverableTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  return entry.recoveryState === "recoverable" || entry.recoveryState === "recovered";
}

export function reconcileNativeTranscriptHistoryEntries(
  current: TranscriptHistoryEntry[],
  nativeEntries: TranscriptHistoryEntry[],
  hiddenOutputPaths: string[],
) {
  replaceNativeTranscriptHistoryTrust(nativeEntries);
  const hidden = new Set(
    normalizeHiddenTranscriptHistory(hiddenOutputPaths).map(transcriptPathIdentity),
  );
  const visibleNative = nativeEntries.filter(
    (entry) => !hidden.has(transcriptPathIdentity(entry.outputPath)),
  );
  const legacyEntries = legacyTranscriptHistoryEntries(current, nativeEntries);
  return normalizeTranscriptHistory([
    ...visibleNative,
    ...legacyEntries,
  ]);
}

export function legacyTranscriptHistoryEntries(
  current: TranscriptHistoryEntry[],
  nativeEntries: TranscriptHistoryEntry[],
) {
  const nativeOutputs = new Set(
    nativeEntries.map((entry) => transcriptPathIdentity(entry.outputPath)),
  );
  const nativeSessions = new Set(
    nativeEntries.flatMap((entry) => entry.sessionId ?? []),
  );
  return normalizeTranscriptHistory(current.filter((entry) => (
    entry.origin === undefined
    && !isCanonicalYapLiveHistoryEntry(entry)
    && !isPreReleaseLiveHistoryEntry(entry)
    && !nativeOutputs.has(transcriptPathIdentity(entry.outputPath))
    && (!entry.sessionId || !nativeSessions.has(entry.sessionId))
  )));
}

export function removeTranscriptHistory(entries: TranscriptHistoryEntry[], outputPath: string) {
  const identity = transcriptPathIdentity(outputPath);
  for (const entry of entries) {
    if (transcriptPathIdentity(entry.outputPath) === identity) {
      revokeNativeTranscriptHistoryEntry(entry);
    }
  }
  return entries.filter((entry) => transcriptPathIdentity(entry.outputPath) !== identity);
}

export function hideTranscriptHistory(outputPaths: string[], outputPath: string) {
  return normalizeHiddenTranscriptHistory([outputPath, ...outputPaths]);
}

function validHistorySessionId(entry: TranscriptHistoryEntry) {
  const sessionId = entry.sessionId;
  return sessionId && /^[a-z0-9_-]{1,128}$/i.test(sessionId) ? sessionId : undefined;
}

function historyArtifactPath(path: string) {
  const normalized = path.replace(/\\/g, "/").toLowerCase();
  const name = normalized.split("/").pop() ?? "";
  return { directory: normalized.slice(0, -name.length), name };
}

export function savedLiveSessionActionIdentity(
  entry: TranscriptHistoryEntry,
): SavedLiveSessionActionIdentity | undefined {
  if (!isNativeLiveTranscriptHistoryEntry(entry)) return undefined;
  const sessionId = validHistorySessionId(entry);
  const expectedCaptureCommitPath = entry.captureCommitPath;
  if (!sessionId || !expectedCaptureCommitPath || isRecoverableTranscriptHistoryEntry(entry)) {
    return undefined;
  }

  const stem = `live-${sessionId}`.toLowerCase();
  const output = historyArtifactPath(entry.outputPath);
  const source = historyArtifactPath(entry.sourcePath);
  const commit = historyArtifactPath(expectedCaptureCommitPath);
  if (
    ![`${stem}.txt`, `${stem}.wav`].includes(output.name)
    || source.name !== `${stem}.wav`
    || commit.name !== `${stem}.commit.json`
    || source.directory !== output.directory
    || commit.directory !== output.directory
  ) return undefined;

  return {
    expectedCaptureCommitPath,
    expectedOutputPath: entry.outputPath,
    sessionId,
  };
}

export function recoverableLiveSessionActionIdentity(
  entry: TranscriptHistoryEntry,
): RecoverableLiveSessionActionIdentity | undefined {
  if (!isNativeLiveTranscriptHistoryEntry(entry)) return undefined;
  const sessionId = validHistorySessionId(entry);
  if (!sessionId || !isRecoverableTranscriptHistoryEntry(entry)) return undefined;
  const artifact = historyArtifactPath(entry.sourcePath);
  const output = historyArtifactPath(entry.outputPath);
  const stem = `live-${sessionId}`.toLowerCase();
  if (![`${stem}.wav.part`, `${stem}.capture.journal.part`, `${stem}.wav`].includes(artifact.name)) {
    return undefined;
  }
  if (
    artifact.directory !== output.directory
    || (entry.recoveryState === "recoverable" && entry.sourcePath !== entry.outputPath)
    || ![artifact.name, `${stem}.txt`].includes(output.name)
  ) return undefined;
  return { expectedArtifactPath: entry.sourcePath, sessionId };
}

export function canDeleteTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  return savedLiveSessionActionIdentity(entry) !== undefined;
}

export function historyEntryPlaybackPath(entry: TranscriptHistoryEntry) {
  return savedLiveSessionActionIdentity(entry) ? entry.sourcePath : undefined;
}

export function savedSessionToTranscriptHistoryEntry(session: SavedTranscriptSession): TranscriptHistoryEntry {
  const createdAt = Number.isFinite(session.createdAtMs) && session.createdAtMs > 0
    ? new Date(session.createdAtMs).toISOString()
    : new Date().toISOString();

  return trustNativeTranscriptHistoryEntry({
    captureCommitPath: session.captureCommitPath ?? undefined,
    createdAt,
    name: session.name,
    origin: session.origin ?? (
      session.captureCommitPath || session.recoveryState ? "live" : undefined
    ),
    outputPath: session.outputPath,
    sessionId: session.sessionId,
    sourcePath: session.sourcePath,
    warning: session.warning ?? undefined,
    recoveryState: session.recoveryState ?? undefined,
  });
}
