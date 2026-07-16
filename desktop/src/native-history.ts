import {
  isCanonicalYapLiveHistoryEntry,
  isPreReleaseLiveHistoryEntry,
  maxTranscriptHistoryEntries,
  normalizeHiddenTranscriptHistory,
  normalizeTranscriptHistory,
  transcriptPathIdentity,
  type TranscriptHistoryEntry,
} from "@/history-model";

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

export type SavedLiveSessionActionIdentity = {
  expectedCaptureCommitPath: string;
  expectedOutputPath: string;
  sessionId: string;
};

export type RecoverableLiveSessionActionIdentity = {
  expectedArtifactPath: string;
  sessionId: string;
};

const trustedNativeHistoryIdentities = new Map<
  string,
  { createdAtMs: number; outputIdentity: string; sessionId: string }
>();

function validHistorySessionId(entry: TranscriptHistoryEntry) {
  const sessionId = entry.sessionId;
  return sessionId && /^[a-z0-9_-]{1,128}$/i.test(sessionId) ? sessionId : undefined;
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
