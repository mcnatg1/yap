import { afterEach, describe, expect, it, vi } from "vitest";

import { createImportedRecordingQueueOwner } from "@/hooks/use-imported-recording-queue";
import type { PlaybackAdmission, RecordingJobView } from "@/lib/app-types";
import { playbackAdmissionDeadlineMs } from "@/lib/playback-registry";
import { createQueuedServerRecordingJobs, maxStoredQueueJobs } from "@/recording-queue";

function deferred<T>() {
  let reject!: (error: unknown) => void;
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    reject = rejectPromise;
    resolve = resolvePromise;
  });
  return { promise, reject, resolve };
}

function admission(label: string): PlaybackAdmission {
  return { byteLength: label.length, playbackPath: `media://${label}` };
}

function queueHarness(initial: RecordingJobView[] = []) {
  let queue = initial;
  const releasePlaybackPaths = vi.fn(async () => undefined);
  const setQueue = vi.fn((next: RecordingJobView[]) => {
    queue = next;
  });
  const warn = vi.fn();
  return {
    get queue() {
      return queue;
    },
    getQueue: () => queue,
    releasePlaybackPaths,
    setQueue,
    warn,
  };
}

describe("imported recording queue ownership", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("commits a quick admission without waiting for a slower item in the batch", async () => {
    const quick = deferred<PlaybackAdmission>();
    const slow = deferred<PlaybackAdmission>();
    const harness = queueHarness();
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath: (path) => path.endsWith("quick.wav") ? quick.promise : slow.promise,
    });

    const adding = owner.addPaths(["C:/quick.wav", "C:/slow.wav"]);
    quick.resolve(admission("quick"));
    await vi.waitFor(() => expect(harness.queue.map((item) => item.path)).toEqual([
      "C:/quick.wav",
    ]));

    slow.resolve(admission("slow"));
    await expect(adding).resolves.toBe(2);
    expect(harness.queue.map((item) => item.path)).toEqual([
      "C:/quick.wav",
      "C:/slow.wav",
    ]);
  });

  it("bounds playback admissions across an imported batch", async () => {
    const admissions = Array.from({ length: 5 }, () => deferred<PlaybackAdmission>());
    const allowPlaybackPath = vi.fn((path: string) => {
      const index = Number(path.match(/(\d+)\.wav$/)?.[1]);
      return admissions[index].promise;
    });
    const harness = queueHarness();
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath,
      maxAdmissionConcurrency: 2,
    });

    const adding = owner.addPaths(Array.from(
      { length: 5 },
      (_, index) => `C:/recording-${index}.wav`,
    ));
    expect(allowPlaybackPath).toHaveBeenCalledTimes(2);

    admissions[0].resolve(admission("recording-0"));
    await vi.waitFor(() => expect(allowPlaybackPath).toHaveBeenCalledTimes(3));
    admissions[1].resolve(admission("recording-1"));
    admissions[2].resolve(admission("recording-2"));
    await vi.waitFor(() => expect(allowPlaybackPath).toHaveBeenCalledTimes(5));
    admissions[3].resolve(admission("recording-3"));
    admissions[4].resolve(admission("recording-4"));

    await expect(adding).resolves.toBe(5);
  });

  it("expires a queued import without starting it after the original deadline", async () => {
    vi.useFakeTimers();
    const admissions = Array.from(
      { length: 4 },
      () => deferred<PlaybackAdmission>(),
    );
    let cursor = 0;
    const allowPlaybackPath = vi.fn(() => admissions[cursor++].promise);
    const harness = queueHarness();
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath,
    });
    const adding = owner.addPaths(Array.from(
      { length: 5 },
      (_, index) => `C:/queued-${index}.wav`,
    ));
    let settled = false;
    void adding.then(() => {
      settled = true;
    });

    try {
      expect(allowPlaybackPath).toHaveBeenCalledTimes(4);
      await vi.advanceTimersByTimeAsync(playbackAdmissionDeadlineMs);

      expect(allowPlaybackPath).toHaveBeenCalledTimes(4);
      expect(settled).toBe(true);
      admissions.forEach((pending, index) => pending.resolve(admission(`late-${index}`)));
      await vi.waitFor(() => expect(harness.releasePlaybackPaths).toHaveBeenCalledTimes(4));
      await adding;
    } finally {
      admissions.forEach((pending, index) => pending.resolve(admission(`late-${index}`)));
      await adding.catch(() => undefined);
    }
  });

  it("does not resurrect an admission that resolves after clear", async () => {
    const pending = deferred<PlaybackAdmission>();
    const harness = queueHarness();
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath: () => pending.promise,
    });

    const add = owner.addPaths(["C:/meeting.wav"]);
    owner.clear();
    pending.resolve(admission("meeting"));

    await expect(add).resolves.toBeUndefined();
    expect(harness.queue).toEqual([]);
    expect(harness.releasePlaybackPaths).toHaveBeenCalledWith(["media://meeting"]);
  });

  it("commits concurrent imports in request order regardless of resolution order", async () => {
    const first = deferred<PlaybackAdmission>();
    const second = deferred<PlaybackAdmission>();
    const harness = queueHarness();
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath: (path) => path.endsWith("first.wav") ? first.promise : second.promise,
    });

    const addFirst = owner.addPaths(["C:/first.wav"]);
    const addSecond = owner.addPaths(["C:/second.wav"]);
    second.resolve(admission("second"));
    await expect(addSecond).resolves.toBe(2);
    first.resolve(admission("first"));
    await expect(addFirst).resolves.toBe(1);

    expect(harness.queue.map(({ id, path }) => ({ id, path }))).toEqual([
      { id: 1, path: "C:/first.wav" },
      { id: 2, path: "C:/second.wav" },
    ]);
    expect(harness.queue.every((item) => (
      item.route === "serverBatch"
      && item.status === "queued_server"
      && item.progressPercent === undefined
    ))).toBe(true);
  });

  it("releases reservations after admission rejection", async () => {
    const existing = createQueuedServerRecordingJobs(
      Array.from({ length: maxStoredQueueJobs - 1 }, (_, index) => ({
        id: index + 1,
        path: `C:/existing-${index}.wav`,
        playbackPath: `media://existing-${index}`,
      })),
    );
    const allowPlaybackPath = vi.fn()
      .mockRejectedValueOnce(new Error("denied"))
      .mockResolvedValueOnce(admission("accepted"));
    const harness = queueHarness(existing);
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath,
    });

    await expect(owner.addPaths(["C:/denied.wav"])).resolves.toBeUndefined();
    await expect(owner.addPaths(["C:/accepted.wav"])).resolves.toBe(maxStoredQueueJobs + 1);

    expect(allowPlaybackPath).toHaveBeenCalledTimes(2);
    expect(harness.queue).toHaveLength(maxStoredQueueJobs);
    expect(harness.queue.at(-1)?.path).toBe("C:/accepted.wav");
    expect(harness.warn).toHaveBeenCalledWith(
      "Some recordings could not be prepared for playback.",
    );
  });

  it("invalidates pending admissions on unmount without waiting for them", async () => {
    const pending = deferred<PlaybackAdmission>();
    const harness = queueHarness();
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath: () => pending.promise,
    });

    const add = owner.addPaths(["C:/meeting.wav"]);
    owner.dispose();
    pending.resolve(admission("meeting"));

    await expect(add).resolves.toBeUndefined();
    expect(harness.setQueue).not.toHaveBeenCalled();
    expect(harness.releasePlaybackPaths).toHaveBeenCalledWith(["media://meeting"]);
  });

  it.each(["clear", "dispose"] as const)(
    "settles stuck admissions after %s and releases results that arrive after the deadline",
    async (action) => {
      vi.useFakeTimers();
      const pending = Array.from({ length: 4 }, () => deferred<PlaybackAdmission>());
      let cursor = 0;
      const allowPlaybackPath = vi.fn(() => pending[cursor++].promise);
      const harness = queueHarness();
      const owner = createImportedRecordingQueueOwner({
        ...harness,
        allowPlaybackPath,
      });
      const adding = owner.addPaths(Array.from(
        { length: 5 },
        (_, index) => `C:/stuck-${index}.wav`,
      ));
      let settled = false;
      void adding.then(() => {
        settled = true;
      });

      expect(allowPlaybackPath).toHaveBeenCalledTimes(4);
      if (action === "clear") owner.clear();
      else owner.dispose();
      await vi.advanceTimersByTimeAsync(playbackAdmissionDeadlineMs);
      const settledAtDeadline = settled;

      pending.forEach((item, index) => item.resolve(admission(`late-${index}`)));
      await adding;
      await Promise.resolve();

      expect(settledAtDeadline).toBe(true);
      expect(allowPlaybackPath).toHaveBeenCalledTimes(4);
      expect(harness.queue).toEqual([]);
      expect(harness.releasePlaybackPaths).toHaveBeenCalledTimes(4);
    },
  );

  it("reserves bounded slots across concurrent admissions", async () => {
    const existing = createQueuedServerRecordingJobs(
      Array.from({ length: maxStoredQueueJobs - 1 }, (_, index) => ({
        id: index + 1,
        path: `C:/existing-${index}.wav`,
        playbackPath: `media://existing-${index}`,
      })),
    );
    const pending = deferred<PlaybackAdmission>();
    const harness = queueHarness(existing);
    const owner = createImportedRecordingQueueOwner({
      ...harness,
      allowPlaybackPath: () => pending.promise,
    });

    const accepted = owner.addPaths(["C:/last.wav"]);
    await expect(owner.addPaths(["C:/overflow.wav"])).resolves.toBeUndefined();
    pending.resolve(admission("last"));
    await accepted;

    expect(harness.queue).toHaveLength(maxStoredQueueJobs);
    expect(harness.queue.at(-1)?.path).toBe("C:/last.wav");
  });
});
