import { invoke, isTauri } from "@tauri-apps/api/core";

import { audioExts, extension } from "@/lib/media-file";
import type { RecordingJobView } from "@/lib/recording-job";

export const legacyRecordingQueueKey = "yap.recordingQueue.v1";
export const maxStoredQueueJobs = 200;

export type LegacyQueueJob = {
  id: number;
  path: string;
};

type LegacyQueueStorage = Pick<Storage, "getItem">;
type LegacyQueueDiscardStorage = Pick<Storage, "removeItem">;

function isLegacyQueueJob(value: unknown): value is LegacyQueueJob {
  if (!value || typeof value !== "object") return false;
  const item = value as Record<string, unknown>;
  return Number.isSafeInteger(item.id) &&
    Number(item.id) > 0 &&
    typeof item.path === "string" &&
    audioExts.has(extension(item.path));
}

export function parseLegacyRecordingQueue(value: unknown): LegacyQueueJob[] {
  if (!Array.isArray(value)) throw new Error("Legacy recording queue is not an array.");

  const seenPaths = new Set<string>();
  return value
    .filter(isLegacyQueueJob)
    .sort((left, right) => right.id - left.id)
    .filter((item) => {
      if (seenPaths.has(item.path)) return false;
      seenPaths.add(item.path);
      return true;
    })
    .slice(0, maxStoredQueueJobs)
    .sort((left, right) => left.id - right.id)
    .map(({ id, path }) => ({ id, path }));
}

export async function migrateLegacyRecordingQueue(
  storage: LegacyQueueStorage | undefined = globalThis.localStorage,
) {
  if (!storage) return { acknowledged: 0, migrated: false };
  const stored = storage.getItem(legacyRecordingQueueKey);
  if (stored === null) return { acknowledged: 0, migrated: false };

  const jobs = parseLegacyRecordingQueue(JSON.parse(stored));
  throw new Error(
    `Yap found ${jobs.length} recording${jobs.length === 1 ? "" : "s"} in its old browser queue. Automatic pathname migration is disabled. Keep the old queue for reference, or discard it and re-add recordings through Choose recordings.`,
  );
}

export function discardLegacyRecordingQueue(
  storage: LegacyQueueDiscardStorage | undefined = globalThis.localStorage,
) {
  storage?.removeItem(legacyRecordingQueueKey);
}

export async function recordingJobsSnapshot() {
  if (!isTauri()) return [];
  return invoke<RecordingJobView[]>("recording_jobs_snapshot");
}

export async function pickRecordingImports() {
  if (!isTauri()) return [];
  return invoke<RecordingJobView[]>("recording_jobs_pick_imports");
}

export async function cancelRecordingJob(jobId: string) {
  return invoke<RecordingJobView>("recording_job_cancel", { jobId });
}

export async function dismissRecordingJob(jobId: string) {
  return invoke<RecordingJobView>("recording_job_dismiss", { jobId });
}

export async function retryRecordingJob(jobId: string) {
  return invoke<RecordingJobView>("recording_job_retry", { jobId });
}
