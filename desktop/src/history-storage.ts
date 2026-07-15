import {
  filterHiddenTranscriptHistory,
  isPreReleaseLiveHistoryEntry,
  normalizeHiddenTranscriptHistory,
  normalizeTranscriptHistory,
  transcriptPathIdentity,
  type TranscriptHistoryEntry,
} from "@/history-model";

const transcriptHistoryKey = "yap.transcriptHistory.v1";
const hiddenTranscriptHistoryKey = "yap.hiddenTranscriptHistory.v1";
const hiddenPruneBatchSize = 200;

export type HistoryStorage = Pick<Storage, "getItem" | "setItem">;

export type OwnedLiveTranscriptPathResolution = {
  requestedPath: string;
  canonicalPath?: string | null;
  missing: boolean;
};

function withoutNativeHistoryOrigin(entry: TranscriptHistoryEntry): TranscriptHistoryEntry {
  const { origin: _origin, ...storedEntry } = entry;
  return storedEntry;
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

export function readVisibleTranscriptHistory(storage: HistoryStorage | undefined = globalThis.localStorage) {
  if (!storage) return [];
  const history = readTranscriptHistory(storage);
  return filterHiddenTranscriptHistory(
    history.filter((entry) => !isPreReleaseLiveHistoryEntry(entry)),
    readHiddenTranscriptHistory(storage),
  );
}

export function writeTranscriptHistory(
  entries: TranscriptHistoryEntry[],
  storage: HistoryStorage = globalThis.localStorage,
) {
  const legacyEntries = entries
    .filter((entry) => entry.origin === undefined)
    .map(withoutNativeHistoryOrigin);
  storage.setItem(transcriptHistoryKey, JSON.stringify(normalizeTranscriptHistory(legacyEntries)));
}

export function writeHiddenTranscriptHistory(
  outputPaths: string[],
  storage: HistoryStorage = globalThis.localStorage,
) {
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
  const nextHistory = history.filter((entry) => {
    const identity = transcriptPathIdentity(entry.outputPath);
    return !confirmedMissing.has(identity) && !staleAliasHistory.has(identity);
  });
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
