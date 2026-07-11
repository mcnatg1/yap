export type TranscriptHistoryEntry = {
  captureCommitPath?: string;
  name: string;
  sourcePath: string;
  outputPath: string;
  createdAt: string;
  warning?: string;
  recoveryState?: "recoverable" | "recovered";
};

export type SavedTranscriptSession = {
  captureCommitPath?: string | null;
  createdAtMs: number;
  name: string;
  sourcePath: string;
  outputPath: string;
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

const hiddenPruneBatchSize = 200;

function isHistoryEntry(value: unknown): value is TranscriptHistoryEntry {
  if (!value || typeof value !== "object") return false;
  const entry = value as Record<string, unknown>;
  return (
    typeof entry.name === "string" &&
    typeof entry.sourcePath === "string" &&
    typeof entry.outputPath === "string" &&
    typeof entry.createdAt === "string" &&
    (entry.warning === undefined || typeof entry.warning === "string") &&
    (entry.captureCommitPath === undefined || typeof entry.captureCommitPath === "string") &&
    (entry.recoveryState === undefined || entry.recoveryState === "recoverable" || entry.recoveryState === "recovered")
  );
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
    return normalizeTranscriptHistory(JSON.parse(storage.getItem(transcriptHistoryKey) ?? "[]"));
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

export function readVisibleTranscriptHistory(storage: HistoryStorage | undefined = globalThis.localStorage) {
  if (!storage) return [];
  return filterHiddenTranscriptHistory(
    readTranscriptHistory(storage),
    readHiddenTranscriptHistory(storage),
  );
}

export function writeTranscriptHistory(entries: TranscriptHistoryEntry[], storage: HistoryStorage = globalThis.localStorage) {
  storage.setItem(transcriptHistoryKey, JSON.stringify(normalizeTranscriptHistory(entries)));
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
  return Boolean(entry.captureCommitPath || entry.recoveryState);
}

export function isRecoverableTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  return entry.recoveryState === "recoverable";
}

export function reconcileNativeTranscriptHistoryEntries(
  current: TranscriptHistoryEntry[],
  nativeEntries: TranscriptHistoryEntry[],
  hiddenOutputPaths: string[],
) {
  const hidden = new Set(
    normalizeHiddenTranscriptHistory(hiddenOutputPaths).map(transcriptPathIdentity),
  );
  const visibleNative = nativeEntries.filter(
    (entry) => !hidden.has(transcriptPathIdentity(entry.outputPath)),
  );
  return normalizeTranscriptHistory([
    ...visibleNative,
    ...current.filter((entry) => !isNativeLiveTranscriptHistoryEntry(entry)),
  ]);
}

export function removeTranscriptHistory(entries: TranscriptHistoryEntry[], outputPath: string) {
  const identity = transcriptPathIdentity(outputPath);
  return entries.filter((entry) => transcriptPathIdentity(entry.outputPath) !== identity);
}

export function hideTranscriptHistory(outputPaths: string[], outputPath: string) {
  return normalizeHiddenTranscriptHistory([outputPath, ...outputPaths]);
}

export function canDeleteTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
  if (isRecoverableTranscriptHistoryEntry(entry)) return false;
  const output = entry.outputPath.replace(/\\/g, "/").toLowerCase();
  const source = entry.sourcePath.replace(/\\/g, "/").toLowerCase();
  const outputName = output.split("/").pop() ?? "";
  const sourceName = source.split("/").pop() ?? "";
  const outputDir = output.slice(0, -outputName.length);
  const sourceDir = source.slice(0, -sourceName.length);
  const stem = outputName.endsWith(".txt") ? outputName.slice(0, -4) : "";
  return (
    stem.startsWith("live-") &&
    output.includes("/yap/live-recordings/") &&
    entry.name.toLowerCase().startsWith("live-") &&
    (source === output || (sourceDir === outputDir && sourceName === `${stem}.wav`))
  );
}

export function historyEntryPlaybackPath(entry: TranscriptHistoryEntry) {
  if (!canDeleteTranscriptHistoryEntry(entry)) return undefined;
  const output = entry.outputPath.replace(/\\/g, "/").toLowerCase();
  const source = entry.sourcePath.replace(/\\/g, "/").toLowerCase();
  const outputName = output.split("/").pop() ?? "";
  const sourceName = source.split("/").pop() ?? "";
  const outputDir = output.slice(0, -outputName.length);
  const sourceDir = source.slice(0, -sourceName.length);
  const stem = outputName.endsWith(".txt") ? outputName.slice(0, -4) : "";

  return stem && sourceDir === outputDir && sourceName === `${stem}.wav`
    ? entry.sourcePath
    : undefined;
}

export function savedSessionToTranscriptHistoryEntry(session: SavedTranscriptSession): TranscriptHistoryEntry {
  const createdAt = Number.isFinite(session.createdAtMs) && session.createdAtMs > 0
    ? new Date(session.createdAtMs).toISOString()
    : new Date().toISOString();

  return {
    captureCommitPath: session.captureCommitPath ?? undefined,
    createdAt,
    name: session.name,
    outputPath: session.outputPath,
    sourcePath: session.sourcePath,
    warning: session.warning ?? undefined,
    recoveryState: session.recoveryState ?? undefined,
  };
}
