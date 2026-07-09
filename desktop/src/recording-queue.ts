import {
  audioExts,
  basename,
  createInitialPipelineState,
  extension,
  type RecordingJobView,
} from "@/lib/app-types";

const recordingQueueKey = "yap.recordingQueue.v1";
const maxStoredQueueJobs = 200;

type QueueStorage = Pick<Storage, "getItem" | "setItem">;

function isStoredQueuedRecording(value: unknown): value is Pick<RecordingJobView, "error" | "id" | "name" | "path"> {
  if (!value || typeof value !== "object") return false;
  const item = value as Record<string, unknown>;
  return (
    Number.isInteger(item.id) &&
    Number(item.id) > 0 &&
    typeof item.path === "string" &&
    audioExts.has(extension(item.path)) &&
    (item.name === undefined || typeof item.name === "string") &&
    (item.error === undefined || typeof item.error === "string")
  );
}

export function normalizeRecordingQueue(value: unknown): RecordingJobView[] {
  if (!Array.isArray(value)) return [];

  const seen = new Set<string>();
  return value
    .filter(isStoredQueuedRecording)
    .filter((item) => {
      if (seen.has(item.path)) return false;
      seen.add(item.path);
      return true;
    })
    .sort((a, b) => a.id - b.id)
    .slice(0, maxStoredQueueJobs)
    .map((item) => ({
      error: item.error,
      id: item.id,
      intent: "recording",
      name: item.name || basename(item.path),
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

export function nextRecordingQueueId(jobs: RecordingJobView[]) {
  return jobs.reduce((next, job) => Math.max(next, job.id + 1), 1);
}
