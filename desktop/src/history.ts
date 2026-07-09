export type TranscriptHistoryEntry = {
  name: string;
  sourcePath: string;
  outputPath: string;
  createdAt: string;
  warning?: string;
};

const transcriptHistoryKey = "yap.transcriptHistory.v1";
const hiddenTranscriptHistoryKey = "yap.hiddenTranscriptHistory.v1";
export const maxTranscriptHistoryEntries = 500;

type HistoryStorage = Pick<Storage, "getItem" | "setItem">;

function isHistoryEntry(value: unknown): value is TranscriptHistoryEntry {
  if (!value || typeof value !== "object") return false;
  const entry = value as Record<string, unknown>;
  return (
    typeof entry.name === "string" &&
    typeof entry.sourcePath === "string" &&
    typeof entry.outputPath === "string" &&
    typeof entry.createdAt === "string" &&
    (entry.warning === undefined || typeof entry.warning === "string")
  );
}

function normalizeTranscriptHistory(value: unknown) {
  if (!Array.isArray(value)) return [];

  const seen = new Set<string>();
  return value
    .filter(isHistoryEntry)
    .filter((entry) => {
      if (seen.has(entry.outputPath)) return false;
      seen.add(entry.outputPath);
      return true;
    })
    .sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))
    .slice(0, maxTranscriptHistoryEntries);
}

export function normalizeHiddenTranscriptHistory(value: unknown) {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.filter((item): item is string => typeof item === "string"))];
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
  const hidden = new Set(normalizeHiddenTranscriptHistory(outputPaths));
  if (!hidden.size) return entries;
  return entries.filter((entry) => !hidden.has(entry.outputPath));
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

export function recordTranscriptHistory(entries: TranscriptHistoryEntry[], entry: TranscriptHistoryEntry) {
  return normalizeTranscriptHistory([entry, ...entries.filter((item) => item.outputPath !== entry.outputPath)]);
}

export function removeTranscriptHistory(entries: TranscriptHistoryEntry[], outputPath: string) {
  return entries.filter((entry) => entry.outputPath !== outputPath);
}

export function hideTranscriptHistory(outputPaths: string[], outputPath: string) {
  return normalizeHiddenTranscriptHistory([outputPath, ...outputPaths]);
}

export function canDeleteTranscriptHistoryEntry(entry: TranscriptHistoryEntry) {
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
