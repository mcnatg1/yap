import { invoke, isTauri } from "@tauri-apps/api/core";

import { audioExts, extension, type RecordingJobView } from "@/lib/app-types";

export const legacyRecordingQueueKey = "yap.recordingQueue.v1";
export const maxStoredQueueJobs = 200;

export type LegacyQueueJob = {
  id: number;
  path: string;
};

export type LegacyQueueImport = {
  schemaVersion: 1;
  jobs: LegacyQueueJob[];
};

export type LegacyImportAcknowledgement = {
  legacyId: number;
  jobId: string;
};

export type LegacyImportRejection = {
  legacyId: number;
  code: string;
  message: string;
};

export type LegacyImportResult = {
  accepted: LegacyImportAcknowledgement[];
  duplicates: LegacyImportAcknowledgement[];
  rejected: LegacyImportRejection[];
};

type LegacyQueueStorage = Pick<Storage, "getItem" | "removeItem">;
type LegacyImporter = (payload: LegacyQueueImport) => Promise<LegacyImportResult>;

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

function acknowledgementIds(result: LegacyImportResult) {
  return [
    ...result.accepted.map(({ legacyId }) => legacyId),
    ...result.duplicates.map(({ legacyId }) => legacyId),
    ...result.rejected.map(({ legacyId }) => legacyId),
  ].sort((left, right) => left - right);
}

function assertCompleteAcknowledgement(
  jobs: LegacyQueueJob[],
  result: LegacyImportResult,
) {
  if (
    !result ||
    !Array.isArray(result.accepted) ||
    !Array.isArray(result.duplicates) ||
    !Array.isArray(result.rejected)
  ) {
    throw new Error("Rust returned an invalid legacy queue acknowledgement.");
  }
  const expected = jobs.map(({ id }) => id).sort((left, right) => left - right);
  const acknowledged = acknowledgementIds(result);
  if (
    expected.length !== acknowledged.length ||
    expected.some((id, index) => id !== acknowledged[index])
  ) {
    throw new Error("Rust did not acknowledge every legacy recording queue row.");
  }
}

async function invokeLegacyImport(payload: LegacyQueueImport) {
  return invoke<LegacyImportResult>("recording_jobs_import_legacy", { payload });
}

export async function migrateLegacyRecordingQueue(
  storage: LegacyQueueStorage | undefined = globalThis.localStorage,
  importLegacy: LegacyImporter = invokeLegacyImport,
) {
  if (!storage) return { acknowledged: 0, migrated: false };
  const stored = storage.getItem(legacyRecordingQueueKey);
  if (stored === null) return { acknowledged: 0, migrated: false };

  const jobs = parseLegacyRecordingQueue(JSON.parse(stored));
  const result = await importLegacy({ schemaVersion: 1, jobs });
  assertCompleteAcknowledgement(jobs, result);
  storage.removeItem(legacyRecordingQueueKey);
  return { acknowledged: jobs.length, migrated: true, result };
}

export async function recordingJobsSnapshot() {
  if (!isTauri()) return [];
  return invoke<RecordingJobView[]>("recording_jobs_snapshot");
}

export async function createRecordingImports(paths: string[]) {
  if (!isTauri()) return [];
  return invoke<RecordingJobView[]>("recording_jobs_create_imports", { paths });
}

export async function cancelRecordingJob(jobId: string) {
  return invoke<RecordingJobView>("recording_job_cancel", { jobId });
}

export async function retryRecordingJob(jobId: string) {
  return invoke<RecordingJobView>("recording_job_retry", { jobId });
}
