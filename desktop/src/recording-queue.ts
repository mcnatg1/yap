import {
  audioExts,
  basename,
  createInitialPipelineState,
  extension,
  queuedServerMessage,
  type QueuedRecordingPath,
  type RecordingJobView,
} from "@/lib/app-types";

const recordingQueueKey = "yap.recordingQueue.v1";
export const maxStoredQueueJobs = 200;

type QueueStorage = Pick<Storage, "getItem" | "setItem">;
type ApprovedQueuedRecordingPath = QueuedRecordingPath & { playbackPath: string };

function isStoredQueuedRecording(value: unknown): value is Pick<RecordingJobView, "id" | "path"> {
  if (!value || typeof value !== "object") return false;
  const item = value as Record<string, unknown>;
  return (
    Number.isSafeInteger(item.id) &&
    Number(item.id) > 0 &&
    typeof item.path === "string" &&
    audioExts.has(extension(item.path))
  );
}

export function normalizeRecordingQueue(value: unknown): RecordingJobView[] {
  if (!Array.isArray(value)) return [];

  const seen = new Set<string>();
  return value
    .filter(isStoredQueuedRecording)
    .sort((a, b) => b.id - a.id)
    .filter((item) => {
      if (seen.has(item.path)) return false;
      seen.add(item.path);
      return true;
    })
    .slice(0, maxStoredQueueJobs)
    .sort((a, b) => a.id - b.id)
    .map((item, index) => ({
      error: queuedServerMessage,
      id: index + 1,
      intent: "recording",
      name: basename(item.path),
      path: item.path,
      pipeline: createInitialPipelineState(),
      route: "serverBatch",
      status: "queued_server",
    }));
}

export function readRecordingQueue(storage: QueueStorage | undefined = globalThis.localStorage) {
  if (!storage) return [];

  try {
    return normalizeRecordingQueue(JSON.parse(storage.getItem(recordingQueueKey) ?? "[]"));
  } catch {
    return [];
  }
}

export function writeRecordingQueue(jobs: RecordingJobView[], storage: QueueStorage = globalThis.localStorage) {
  const queued = jobs.filter((job) => job.intent === "recording" && job.route === "serverBatch" && job.status === "queued_server");
  storage.setItem(recordingQueueKey, JSON.stringify(normalizeRecordingQueue(queued)));
}

export function availableQueuedServerSlots(jobs: RecordingJobView[]) {
  const queued = jobs.filter((job) => job.intent === "recording" && job.route === "serverBatch" && job.status === "queued_server");
  return Math.max(0, maxStoredQueueJobs - queued.length);
}

export function createQueuedServerRecordingJobs(
  recordings: ApprovedQueuedRecordingPath[],
): RecordingJobView[] {
  return recordings.map(({ id, path, playbackPath }) => ({
    error: queuedServerMessage,
    id,
    intent: "recording",
    name: basename(path),
    path,
    playbackPath,
    pipeline: createInitialPipelineState(),
    route: "serverBatch",
    status: "queued_server",
  }));
}

export function nextRecordingQueueId(jobs: RecordingJobView[]) {
  return jobs.reduce((next, job) => Math.max(next, job.id + 1), 1);
}
