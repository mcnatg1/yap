import { describe, expect, it } from "vitest";

import {
  createInitialPipelineState,
  isRecordingActive,
  queuedServerMessage,
  type RecordingJobStatus,
  type RecordingJobView,
} from "@/lib/app-types";
import {
  availableQueuedServerSlots,
  createQueuedServerRecordingJobs,
  maxStoredQueueJobs,
  nextRecordingQueueId,
  normalizeRecordingQueue,
  readRecordingQueue,
  writeRecordingQueue,
} from "@/recording-queue";

function storageWith(value: string) {
  const storage = { value };
  return {
    getItem: () => storage.value,
    setItem(_key: string, next: string) {
      storage.value = next;
    },
  };
}

describe("recording queue storage", () => {
  it("hydrates queued server recordings with compact safe ids", () => {
    const jobs = normalizeRecordingQueue([
      { id: 3, name: "meeting.wav", path: "C:/meeting.wav", error: "Queued" },
      { id: 4, path: "C:/notes.txt" },
      { id: 5, path: "C:/meeting.wav" },
      { id: 6, path: "C:/demo.mp3" },
    ]);

    expect(jobs).toMatchObject([
      { error: queuedServerMessage, id: 1, path: "C:/meeting.wav", route: "serverBatch", status: "queued_server" },
      { error: queuedServerMessage, id: 2, name: "demo.mp3", path: "C:/demo.mp3", route: "serverBatch", status: "queued_server" },
    ]);
    expect(nextRecordingQueueId(jobs)).toBe(3);
  });

  it("repairs duplicate ids without dropping path-unique recordings", () => {
    const jobs = normalizeRecordingQueue([
      { id: 7, path: "C:/first.wav" },
      { id: 7, path: "C:/second.wav" },
      { id: 8, path: "C:/third.wav" },
    ]);

    expect(jobs.map((job) => job.id)).toEqual([1, 2, 3]);
    expect(jobs.map((job) => job.path)).toEqual([
      "C:/first.wav",
      "C:/second.wav",
      "C:/third.wav",
    ]);
    expect(new Set(jobs.map((job) => job.id)).size).toBe(jobs.length);
  });

  it("rejects unsafe persisted ids before assigning unique safe ids", () => {
    const jobs = normalizeRecordingQueue([
      { id: Number.MAX_SAFE_INTEGER, path: "C:/safe-max.wav" },
      { id: Number.MAX_SAFE_INTEGER + 1, path: "C:/unsafe.wav" },
      { id: Number.POSITIVE_INFINITY, path: "C:/infinite.wav" },
      { id: 1.5, path: "C:/fractional.wav" },
      { id: 1, path: "C:/normal.wav" },
    ]);

    expect(jobs.map(({ id, path }) => ({ id, path }))).toEqual([
      { id: 1, path: "C:/normal.wav" },
      { id: 2, path: "C:/safe-max.wav" },
    ]);
    expect(jobs.every((job) => Number.isSafeInteger(job.id))).toBe(true);
    expect(nextRecordingQueueId(jobs)).toBe(3);
  });

  it("stores only queued server jobs", () => {
    const storage = storageWith("[]");
    const queued: RecordingJobView = {
      error: "Server queued",
      id: 2,
      intent: "recording",
      name: "take.wav",
      path: "C:/take.wav",
      playbackPath: "C:/take.wav",
      pipeline: createInitialPipelineState(),
      route: "serverBatch",
      status: "queued_server",
    };

    writeRecordingQueue([
      queued,
      { ...queued, id: 3, path: "C:/done.wav", status: "complete" },
    ], storage);

    expect(readRecordingQueue(storage)).toMatchObject([
      { id: 1, path: "C:/take.wav", status: "queued_server" },
    ]);
    expect(readRecordingQueue(storage)[0].playbackPath).toBeUndefined();
  });

  it("bounds persisted queue payloads", () => {
    const jobs = normalizeRecordingQueue(
      Array.from({ length: 205 }, (_, index) => ({
        id: index + 1,
        path: `C:/take-${index + 1}.wav`,
      })).reverse(),
    );

    expect(jobs).toHaveLength(200);
    expect(jobs[0]).toMatchObject({ id: 1, path: "C:/take-6.wav" });
    expect(jobs.at(-1)).toMatchObject({ id: 200, path: "C:/take-205.wav" });
  });

  it("round-trips only the newest bounded queued-server payload", () => {
    const storage = storageWith("[]");
    const jobs = createQueuedServerRecordingJobs(
      Array.from({ length: maxStoredQueueJobs + 5 }, (_, index) => ({
        id: index + 1,
        path: `C:/take-${index + 1}.wav`,
        playbackPath: `http://127.0.0.1/media/${index + 1}`,
      })),
    );

    writeRecordingQueue(jobs, storage);

    const restored = readRecordingQueue(storage);
    expect(restored).toHaveLength(maxStoredQueueJobs);
    expect(restored[0].path).toBe("C:/take-6.wav");
    expect(restored.at(-1)?.path).toBe("C:/take-205.wav");
    expect(restored.every((job) => job.route === "serverBatch" && job.status === "queued_server"))
      .toBe(true);
  });

  it("projects approved selected recordings into queued server jobs", () => {
    const jobs = createQueuedServerRecordingJobs(
      [{ id: 9, path: "C:/meeting.wav", playbackPath: "\\\\?\\C:\\meeting.wav" }],
    );

    expect(jobs).toMatchObject([
      {
        error: queuedServerMessage,
        id: 9,
        intent: "recording",
        name: "meeting.wav",
        path: "C:/meeting.wav",
        playbackPath: "\\\\?\\C:\\meeting.wav",
        route: "serverBatch",
        status: "queued_server",
      },
    ]);
    expect(jobs[0].pipeline).toEqual(createInitialPipelineState());
  });

  it("counts only queued server recordings against persisted queue slots", () => {
    const queued = createQueuedServerRecordingJobs(
      Array.from({ length: 2 }, (_, index) => ({
        id: index + 1,
        path: `C:/meeting-${index}.wav`,
        playbackPath: `C:/meeting-${index}.wav`,
      })),
    );

    expect(availableQueuedServerSlots([
      ...queued,
      { ...queued[0], id: 10, status: "complete" },
    ])).toBe(maxStoredQueueJobs - 2);
  });

  it("uses a model-agnostic server status and truthful queue copy", () => {
    const serverProcessingStatus: RecordingJobStatus = "server_processing";

    expect(isRecordingActive(serverProcessingStatus)).toBe(true);
    expect(queuedServerMessage).toContain("organization's transcription server");
    expect(queuedServerMessage).not.toMatch(/\blocal\b|\bprivate\b/i);
  });
});
