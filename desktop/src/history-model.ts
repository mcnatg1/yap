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

export const maxTranscriptHistoryEntries = 500;

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

export function normalizeTranscriptHistory(value: unknown) {
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
  return value
    .filter((item): item is string => {
      if (typeof item !== "string") return false;
      const identity = transcriptPathIdentity(item);
      if (seen.has(identity)) return false;
      seen.add(identity);
      return true;
    })
    .slice(0, maxTranscriptHistoryEntries);
}

export function filterHiddenTranscriptHistory(entries: TranscriptHistoryEntry[], outputPaths: string[]) {
  const hidden = new Set(
    normalizeHiddenTranscriptHistory(outputPaths).map(transcriptPathIdentity),
  );
  if (!hidden.size) return entries;
  return entries.filter((entry) => !hidden.has(transcriptPathIdentity(entry.outputPath)));
}

export function filterLegacyHiddenTranscriptHistory(
  entries: TranscriptHistoryEntry[],
  outputPaths: string[],
) {
  const hidden = new Set(
    normalizeHiddenTranscriptHistory(outputPaths).map(transcriptPathIdentity),
  );
  if (!hidden.size) return entries;
  return entries.filter((entry) => (
    entry.origin !== undefined || !hidden.has(transcriptPathIdentity(entry.outputPath))
  ));
}

function historyPathBasename(path: string) {
  return path.replace(/\\/g, "/").split("/").pop()?.toLowerCase() ?? "";
}

export function isPreReleaseLiveHistoryEntry(entry: TranscriptHistoryEntry) {
  const name = entry.name.toLowerCase();
  return /^live-\d+(?:-\d+)?$/.test(name)
    && historyPathBasename(entry.outputPath) === `${name}.txt`
    && historyPathBasename(entry.sourcePath) === `${name}.wav`;
}

export function isCanonicalYapLiveHistoryEntry(entry: TranscriptHistoryEntry) {
  return Boolean(entry.captureCommitPath || entry.recoveryState);
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

export function hideTranscriptHistory(outputPaths: string[], outputPath: string) {
  return normalizeHiddenTranscriptHistory([outputPath, ...outputPaths]);
}
