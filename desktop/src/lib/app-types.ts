export type WorkspaceView = "home" | "transcribe" | "transcripts" | "polish";

export type RailAction = WorkspaceView | "details" | "help";

export const workspaceCopy: Record<WorkspaceView, { title: string; description: string }> = {
  home: {
    title: "Welcome back",
    description: "Recent transcripts and a quick path back into transcribing.",
  },
  transcribe: {
    title: "Transcribe",
    description: "Add audio or video files, transcribe locally, and review the text here.",
  },
  transcripts: {
    title: "Transcripts",
    description: "Past transcriptions, grouped by day.",
  },
  polish: {
    title: "Polish",
    description: "Clean up the selected transcript before you copy or export it.",
  },
};

export const acceptedFormats = "MP3, M4A, WAV, MP4, FLAC, OGG, WEBM";

export const audioExtensions = ["mp3", "m4a", "wav", "mp4", "flac", "ogg", "webm"];
export const audioExts = new Set(audioExtensions.map((format) => `.${format}`));

export function basename(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

export function extension(path: string) {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot).toLowerCase();
}

export function formatHistoryDate(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Saved";

  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    month: "short",
  }).format(date);
}

function localDayKey(date: Date) {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

export function historyEntryTime(entry: { createdAt: string }) {
  const time = Date.parse(entry.createdAt);
  return Number.isFinite(time) ? time : 0;
}

function historyDayLabel(date: Date) {
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  const key = localDayKey(date);
  if (key === localDayKey(today)) return "Today";
  if (key === localDayKey(yesterday)) return "Yesterday";
  return new Intl.DateTimeFormat(undefined, {
    weekday: "long",
    month: "short",
    day: "numeric",
  }).format(date);
}

export function formatElapsed(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return `${minutes}:${String(remainder).padStart(2, "0")}`;
}

export function formatHistoryTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Saved";

  return new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

export function groupHistoryByDay<T extends { createdAt: string }>(entries: T[]) {
  const sorted = [...entries].sort((a, b) => historyEntryTime(b) - historyEntryTime(a));
  const groups: { key: string; label: string; entries: T[] }[] = [];
  const indexByKey = new Map<string, number>();

  for (const entry of sorted) {
    const date = new Date(entry.createdAt);
    const key = Number.isNaN(date.getTime()) ? "unknown" : localDayKey(date);
    const label = Number.isNaN(date.getTime()) ? "Earlier" : historyDayLabel(date);
    const index = indexByKey.get(key);

    if (index === undefined) {
      indexByKey.set(key, groups.length);
      groups.push({ key, label, entries: [entry] });
    } else {
      groups[index].entries.push(entry);
    }
  }

  return groups;
}
