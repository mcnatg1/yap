import { describe, expect, it } from "vitest";

import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";
import { nextRecordingQueueId, normalizeRecordingQueue, readRecordingQueue, writeRecordingQueue } from "@/recording-queue";

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
  it("hydrates only queued server recordings and resumes ids", () => {
    const jobs = normalizeRecordingQueue([
      { id: 3, name: "meeting.wav", path: "C:/meeting.wav", error: "Queued" },
      { id: 4, path: "C:/notes.txt" },
      { id: 5, path: "C:/meeting.wav" },
      { id: 6, path: "C:/demo.mp3" },
    ]);

    expect(jobs).toMatchObject([
      { id: 5, path: "C:/meeting.wav", route: "serverBatch", status: "queued_server" },
      { id: 6, name: "demo.mp3", path: "C:/demo.mp3", route: "serverBatch", status: "queued_server" },
    ]);
    expect(nextRecordingQueueId(jobs)).toBe(7);
  });

  it("drops duplicate ids from corrupt queue storage", () => {
    const jobs = normalizeRecordingQueue([
      { id: 7, path: "C:/first.wav" },
      { id: 7, path: "C:/second.wav" },
      { id: 8, path: "C:/third.wav" },
    ]);

    expect(jobs.map((job) => job.id)).toEqual([7, 8]);
    expect(new Set(jobs.map((job) => job.id)).size).toBe(jobs.length);
  });

  it("stores only queued server jobs", () => {
    const storage = storageWith("[]");
    const queued: RecordingJobView = {
      error: "Server queued",
      id: 2,
      intent: "recording",
      name: "take.wav",
      path: "C:/take.wav",
      pipeline: createInitialPipelineState(),
      route: "serverBatch",
      status: "queued_server",
    };

    writeRecordingQueue([
      queued,
      { ...queued, id: 3, path: "C:/done.wav", status: "complete" },
    ], storage);

    expect(readRecordingQueue(storage)).toMatchObject([
      { id: 2, path: "C:/take.wav", status: "queued_server" },
    ]);
  });

  it("bounds persisted queue payloads", () => {
    const jobs = normalizeRecordingQueue(
      Array.from({ length: 205 }, (_, index) => ({
        id: index + 1,
        path: `C:/take-${index + 1}.wav`,
      })).reverse(),
    );

    expect(jobs).toHaveLength(200);
    expect(jobs[0].id).toBe(6);
    expect(jobs.at(-1)?.id).toBe(205);
  });
});
