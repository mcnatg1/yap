import { describe, expect, it, vi } from "vitest";

import {
  legacyRecordingQueueKey,
  maxStoredQueueJobs,
  migrateLegacyRecordingQueue,
  parseLegacyRecordingQueue,
} from "@/recording-queue";

function storageWith(value: string | null) {
  let current = value;
  return {
    getItem: vi.fn(() => current),
    removeItem: vi.fn(() => {
      current = null;
    }),
  };
}

describe("recording queue migration", () => {
  it("parses only supported legacy rows and bounds the newest 200 without renumbering", () => {
    const parsed = parseLegacyRecordingQueue([
      { id: 1, path: "C:/old.wav" },
      { id: 2, path: "C:/notes.txt" },
      ...Array.from({ length: 205 }, (_, index) => ({
        id: index + 3,
        path: `C:/take-${index + 1}.wav`,
      })),
    ]);

    expect(parsed).toHaveLength(maxStoredQueueJobs);
    expect(parsed[0]).toEqual({ id: 8, path: "C:/take-6.wav" });
    expect(parsed.at(-1)).toEqual({ id: 207, path: "C:/take-205.wav" });
  });

  it("removes localStorage only after Rust acknowledges every row", async () => {
    const storage = storageWith(JSON.stringify([
      { id: 7, path: "C:/accepted.wav" },
      { id: 8, path: "C:/duplicate.wav" },
      { id: 9, path: "C:/missing.wav" },
    ]));
    const importLegacy = vi.fn(async () => ({
      accepted: [{ legacyId: 7, jobId: "legacy-7-a" }],
      duplicates: [{ legacyId: 8, jobId: "job-existing" }],
      rejected: [{ legacyId: 9, code: "SOURCE_MISSING", message: "Missing" }],
    }));

    await expect(migrateLegacyRecordingQueue(storage, importLegacy)).resolves.toMatchObject({
      acknowledged: 3,
      migrated: true,
    });
    expect(importLegacy).toHaveBeenCalledWith({
      schemaVersion: 1,
      jobs: [
        { id: 7, path: "C:/accepted.wav" },
        { id: 8, path: "C:/duplicate.wav" },
        { id: 9, path: "C:/missing.wav" },
      ],
    });
    expect(storage.removeItem).toHaveBeenCalledWith(legacyRecordingQueueKey);
  });

  it("retains localStorage when acknowledgement is incomplete or the command fails", async () => {
    const partialStorage = storageWith(JSON.stringify([
      { id: 1, path: "C:/one.wav" },
      { id: 2, path: "C:/two.wav" },
    ]));
    await expect(migrateLegacyRecordingQueue(partialStorage, async () => ({
      accepted: [{ legacyId: 1, jobId: "legacy-1-a" }],
      duplicates: [],
      rejected: [],
    }))).rejects.toThrow(/acknowledge every/i);
    expect(partialStorage.removeItem).not.toHaveBeenCalled();

    const failedStorage = storageWith(JSON.stringify([{ id: 3, path: "C:/three.wav" }]));
    await expect(migrateLegacyRecordingQueue(failedStorage, async () => {
      throw new Error("database unavailable");
    })).rejects.toThrow("database unavailable");
    expect(failedStorage.removeItem).not.toHaveBeenCalled();
  });

  it("does nothing when the one-time legacy key is absent", async () => {
    const storage = storageWith(null);
    const importLegacy = vi.fn();

    await expect(migrateLegacyRecordingQueue(storage, importLegacy)).resolves.toEqual({
      acknowledged: 0,
      migrated: false,
    });
    expect(importLegacy).not.toHaveBeenCalled();
  });
});
