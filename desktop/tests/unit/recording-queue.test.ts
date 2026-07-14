import { describe, expect, it, vi } from "vitest";

import {
  discardLegacyRecordingQueue,
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

  it("keeps the legacy queue and refuses automatic pathname migration", async () => {
    const storage = storageWith(JSON.stringify([
      { id: 7, path: "C:/accepted.wav" },
      { id: 8, path: "C:/second.wav" },
    ]));

    await expect(migrateLegacyRecordingQueue(storage)).rejects.toThrow(
      /automatic pathname migration is disabled/i,
    );
    expect(storage.removeItem).not.toHaveBeenCalled();
    expect(storage.getItem(legacyRecordingQueueKey)).not.toBeNull();
  });

  it.each([
    ["malformed JSON", "{"],
    ["a non-array value", JSON.stringify({ id: 1, path: "C:/one.wav" })],
  ])("retains the legacy key when it contains %s", async (_description, stored) => {
    const storage = storageWith(stored);
    await expect(migrateLegacyRecordingQueue(storage)).rejects.toThrow();

    expect(storage.removeItem).not.toHaveBeenCalled();
    expect(storage.getItem(legacyRecordingQueueKey)).toBe(stored);
  });

  it("explicitly discards only the one-time legacy key", () => {
    const values = new Map([
      [legacyRecordingQueueKey, "malformed"],
      ["yap.unrelated", "keep-me"],
    ]);
    const storage = {
      removeItem: vi.fn((key: string) => values.delete(key)),
    };

    discardLegacyRecordingQueue(storage);

    expect(storage.removeItem).toHaveBeenCalledOnce();
    expect(storage.removeItem).toHaveBeenCalledWith(legacyRecordingQueueKey);
    expect(values.get(legacyRecordingQueueKey)).toBeUndefined();
    expect(values.get("yap.unrelated")).toBe("keep-me");
  });

  it("does nothing when the one-time legacy key is absent", async () => {
    const storage = storageWith(null);

    await expect(migrateLegacyRecordingQueue(storage)).resolves.toEqual({
      acknowledged: 0,
      migrated: false,
    });
  });
});
