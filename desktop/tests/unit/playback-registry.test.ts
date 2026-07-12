import { afterEach, describe, expect, it, vi } from "vitest";

import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";
import {
  applyRestoredQueuePlaybackPaths,
  clearTerminalQueuePlaybackAdmissions,
  createPlaybackAdmissionTracker,
  currentPlaybackPaths,
  maxPlaybackRestoreConcurrency,
  maxWaveformAdmissionBytes,
  mergeHistoryPlaybackAdmissions,
  playbackAdmissionDeadlineMs,
  projectHistoryPlaybackAdmission,
  releaseRecordingPlaybackPaths,
  restoreHistoryPlaybackAdmission,
  restoreQueuePlaybackPaths,
  settlePlaybackAdmissionBeforeDeadline,
  trimHistoryPlaybackAdmissions,
  validatePlaybackAdmission,
} from "@/lib/playback-registry";

function queuedRecording(id: number, path: string): RecordingJobView {
  return {
    id,
    intent: "recording",
    name: path,
    path,
    pipeline: createInitialPipelineState(),
    route: "serverBatch",
    status: "queued_server",
  };
}

function playbackAdmission(id: number) {
  return {
    byteLength: 0,
    playbackPath: `http://127.0.0.1:43123/media/${id.toString(16).padStart(64, "0")}`,
  };
}

describe("playback registry projections", () => {
  it("applies restored playback paths without changing unrelated queue items", () => {
    const first = queuedRecording(1, "C:/meeting.wav");
    const second = queuedRecording(2, "C:/other.wav");

    const next = applyRestoredQueuePlaybackPaths(
      [first, second],
      [{ byteLength: 4_096, id: 1, playbackPath: "\\\\?\\C:\\meeting.wav" }],
    );

    expect(next[0]).toMatchObject({
      playbackByteLength: 4_096,
      playbackPath: "\\\\?\\C:\\meeting.wav",
    });
    expect(next[1]).toBe(second);
  });

  it("preserves exact native lengths without converting unsafe u64 values to number", () => {
    const admission = validatePlaybackAdmission({
      byteLength: "9007199254740993",
      playbackPath: `http://127.0.0.1:43123/media/${"a".repeat(64)}`,
      waveformEligible: true,
    });

    expect(admission.byteLength).toBe(maxWaveformAdmissionBytes + 1);
    expect(() => validatePlaybackAdmission({
      byteLength: 9_007_199_254_740_992,
      playbackPath: `http://127.0.0.1:43123/media/${"b".repeat(64)}`,
      waveformEligible: false,
    })).toThrow("Invalid playback admission");
  });

  it("bounds restore IPC concurrency with a fixed worker count", async () => {
    let active = 0;
    let highWaterMark = 0;
    const restore = vi.fn(async (path: string) => {
      active += 1;
      highWaterMark = Math.max(highWaterMark, active);
      await new Promise((resolve) => setTimeout(resolve, 2));
      active -= 1;
      const id = Number(path.match(/(\d+)/)?.[1] ?? 0);
      return {
        byteLength: 0,
        playbackPath: `http://127.0.0.1:43123/media/${id.toString(16).padStart(64, "0")}`,
      };
    });
    const queue = Array.from({ length: 25 }, (_, index) =>
      queuedRecording(index + 1, `C:/meeting-${index + 1}.wav`));

    const restored = await restoreQueuePlaybackPaths(queue, {
      release: vi.fn(),
      restore,
      runtime: true,
    });

    expect(restore).toHaveBeenCalledTimes(25);
    expect(highWaterMark).toBe(maxPlaybackRestoreConcurrency);
    expect(restored.map((entry) => entry.id)).toEqual(queue.map((entry) => entry.id));
  });

  it("keeps native admission permits until late work and revocation settle", async () => {
    vi.useFakeTimers();
    const nativeAdmissions = Array.from({ length: maxPlaybackRestoreConcurrency }, () =>
      Promise.withResolvers<ReturnType<typeof playbackAdmission>>());
    const lateRelease = Promise.withResolvers<void>();
    let nativeCursor = 0;
    const admit = vi.fn(() => nativeAdmissions[nativeCursor++].promise);
    const release = vi.fn((playbackPath: string) =>
      playbackPath === playbackAdmission(1).playbackPath
        ? lateRelease.promise
        : Promise.resolve());

    let freshAdmissions: Array<PromiseWithResolvers<ReturnType<typeof playbackAdmission>>> = [];
    try {
      const timedOut = Array.from(
        { length: maxPlaybackRestoreConcurrency + 1 },
        () => settlePlaybackAdmissionBeforeDeadline(admit, release),
      );
      expect(admit).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);

      await vi.advanceTimersByTimeAsync(playbackAdmissionDeadlineMs);
      await expect(Promise.all(timedOut)).resolves.toEqual(
        Array(maxPlaybackRestoreConcurrency + 1).fill(undefined),
      );
      expect(admit).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);

      nativeAdmissions[0].resolve(playbackAdmission(1));
      await vi.waitFor(() => expect(release).toHaveBeenCalledWith(
        playbackAdmission(1).playbackPath,
      ));
      freshAdmissions = [
        Promise.withResolvers<ReturnType<typeof playbackAdmission>>(),
        Promise.withResolvers<ReturnType<typeof playbackAdmission>>(),
      ];
      let freshCursor = 0;
      const freshAdmit = vi.fn(() => freshAdmissions[freshCursor++].promise);
      const fresh = [
        settlePlaybackAdmissionBeforeDeadline(freshAdmit, release),
        settlePlaybackAdmissionBeforeDeadline(freshAdmit, release),
      ];
      expect(freshAdmit).not.toHaveBeenCalled();

      lateRelease.resolve();
      await vi.waitFor(() => expect(freshAdmit).toHaveBeenCalledOnce());
      freshAdmissions[0].resolve(playbackAdmission(20));
      await vi.waitFor(() => expect(freshAdmit).toHaveBeenCalledTimes(2));
      freshAdmissions[1].resolve(playbackAdmission(21));
      await expect(Promise.all(fresh)).resolves.toEqual([
        playbackAdmission(20),
        playbackAdmission(21),
      ]);

      nativeAdmissions.slice(1).forEach((pending, index) => {
        pending.resolve(playbackAdmission(index + 2));
      });
      await vi.waitFor(() => expect(release).toHaveBeenCalledTimes(
        maxPlaybackRestoreConcurrency,
      ));
    } finally {
      lateRelease.resolve();
      nativeAdmissions.forEach((pending, index) => {
        pending.resolve(playbackAdmission(index + 1));
      });
      freshAdmissions.forEach((pending, index) => {
        pending.resolve(playbackAdmission(index + 20));
      });
      await Promise.resolve();
      await Promise.resolve();
      vi.useRealTimers();
    }
  });

  it("claims each restored admission while slower peers are still pending", async () => {
    vi.useFakeTimers();
    const trackerRelease = vi.fn(async () => undefined);
    const tracker = createPlaybackAdmissionTracker(trackerRelease, 5_000);
    const claimed = new Set<string>();
    const slow = Promise.withResolvers<ReturnType<typeof playbackAdmission>>();
    const fastAdmission = playbackAdmission(30);
    const slowAdmission = playbackAdmission(31);
    const restore = vi.fn((path: string) => {
      if (path.endsWith("fast.wav")) {
        tracker.track(fastAdmission.playbackPath);
        return Promise.resolve(fastAdmission);
      }
      return slow.promise.then((admission) => {
        tracker.track(admission.playbackPath);
        return admission;
      });
    });
    const options = {
      claim: (playbackPath: string) => {
        claimed.add(playbackPath);
        tracker.reconcile(claimed);
      },
      release: async (playbackPath: string) => tracker.forget(playbackPath),
      restore,
      runtime: true,
    };

    const restoring = restoreQueuePlaybackPaths([
      queuedRecording(1, "C:/fast.wav"),
      queuedRecording(2, "C:/slow.wav"),
    ], options);
    try {
      await Promise.resolve();
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(5_001);

      expect(claimed).toContain(fastAdmission.playbackPath);
      expect(trackerRelease).not.toHaveBeenCalled();

      slow.resolve(slowAdmission);
      await expect(restoring).resolves.toEqual([
        { id: 1, sourcePath: "C:/fast.wav", ...fastAdmission },
        { id: 2, sourcePath: "C:/slow.wav", ...slowAdmission },
      ]);
    } finally {
      slow.resolve(slowAdmission);
      await restoring.catch(() => undefined);
      tracker.dispose();
      vi.useRealTimers();
    }
  });

  it("stops stale restore issuance and releases in-flight admissions", async () => {
    const controller = new AbortController();
    const pending: Array<(value: { byteLength: number; playbackPath: string }) => void> = [];
    const restore = vi.fn(() => new Promise<{ byteLength: number; playbackPath: string }>((resolve) => {
      pending.push(resolve);
    }));
    const release = vi.fn(async () => undefined);
    const queue = Array.from({ length: 40 }, (_, index) =>
      queuedRecording(index + 1, `C:/meeting-${index + 1}.wav`));
    const restoring = restoreQueuePlaybackPaths(queue, {
      release,
      restore,
      runtime: true,
      signal: controller.signal,
    });
    await Promise.resolve();

    expect(restore).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);
    controller.abort();
    pending.forEach((resolve, index) => resolve({
      byteLength: 0,
      playbackPath: `http://127.0.0.1:43123/media/${index.toString(16).padStart(64, "0")}`,
    }));

    await expect(restoring).resolves.toEqual([]);
    expect(restore).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);
    expect(release).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);
  });

  it("shares the restore budget and deduplicates paths across overlapping generations", async () => {
    let active = 0;
    let highWaterMark = 0;
    const restore = vi.fn(async (path: string) => {
      active += 1;
      highWaterMark = Math.max(highWaterMark, active);
      await new Promise((resolve) => setTimeout(resolve, 5));
      active -= 1;
      return {
        byteLength: 0,
        playbackPath: `http://127.0.0.1:43123/media/${path.charCodeAt(path.length - 5).toString(16).padStart(64, "0")}`,
      };
    });
    const firstQueue = Array.from({ length: 6 }, (_, index) =>
      queuedRecording(index + 1, `C:/shared-${index}.wav`));
    const secondQueue = Array.from({ length: 6 }, (_, index) =>
      queuedRecording(index + 20, `C:/shared-${index}.wav`));

    await Promise.all([
      restoreQueuePlaybackPaths(firstQueue, { restore, runtime: true }),
      restoreQueuePlaybackPaths(secondQueue, { restore, runtime: true }),
    ]);

    expect(restore).toHaveBeenCalledTimes(6);
    expect(highWaterMark).toBe(maxPlaybackRestoreConcurrency);
  });

  it("expires queued restores without starting more native work after the deadline", async () => {
    vi.useFakeTimers();
    const pending: Array<(value: { byteLength: number; playbackPath: string }) => void> = [];
    const release = vi.fn(async () => undefined);
    const restore = vi.fn((path: string) => {
      const id = Number(path.match(/(\d+)/)?.[1] ?? 0);
      const admission = {
        byteLength: 0,
        playbackPath: `http://127.0.0.1:43123/media/${id.toString(16).padStart(64, "0")}`,
      };
      return id === maxPlaybackRestoreConcurrency + 1
        ? Promise.resolve(admission)
        : new Promise<typeof admission>((resolve) => pending.push(resolve));
    });
    const queue = Array.from(
      { length: maxPlaybackRestoreConcurrency + 1 },
      (_, index) => queuedRecording(index + 1, `C:/stuck-${index + 1}.wav`),
    );
    const restoring = restoreQueuePlaybackPaths(queue, {
      release,
      restore,
      runtime: true,
    });
    await Promise.resolve();

    expect(restore).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);
    await vi.advanceTimersByTimeAsync(playbackAdmissionDeadlineMs);
    const issuedAfterDeadline = restore.mock.calls.length;
    pending.forEach((resolve, index) => resolve({
      byteLength: 0,
      playbackPath: `http://127.0.0.1:43123/media/${(index + 10).toString(16).padStart(64, "0")}`,
    }));
    const restored = await restoring;
    await Promise.resolve();
    vi.useRealTimers();

    expect(issuedAfterDeadline).toBe(maxPlaybackRestoreConcurrency);
    expect(restored).toEqual([]);
    expect(release).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);
  });

  it("strips failed and cancelled queue capabilities from the active projection", () => {
    const active = {
      ...queuedRecording(1, "C:/active.wav"),
      playbackByteLength: 0,
      playbackPath: `http://127.0.0.1:43123/media/${"1".repeat(64)}`,
    };
    const failed = {
      ...queuedRecording(2, "C:/failed.wav"),
      playbackByteLength: 0,
      playbackPath: `http://127.0.0.1:43123/media/${"2".repeat(64)}`,
      status: "failed" as const,
    };
    const cancelled = {
      ...queuedRecording(3, "C:/cancelled.wav"),
      playbackByteLength: 0,
      playbackPath: `http://127.0.0.1:43123/media/${"3".repeat(64)}`,
      status: "cancelled" as const,
    };

    expect(currentPlaybackPaths([active, failed, cancelled], {})).toEqual([
      active.playbackPath,
    ]);
    expect(clearTerminalQueuePlaybackAdmissions([active, failed, cancelled])).toEqual([
      active,
      expect.not.objectContaining({ playbackPath: expect.anything() }),
      expect.not.objectContaining({ playbackPath: expect.anything() }),
    ]);
  });

  it("bounds bulk release concurrency", async () => {
    let active = 0;
    let highWaterMark = 0;
    const release = vi.fn(async () => {
      active += 1;
      highWaterMark = Math.max(highWaterMark, active);
      await new Promise((resolve) => setTimeout(resolve, 2));
      active -= 1;
    });
    const playbackPaths = Array.from(
      { length: 30 },
      (_, index) => `http://127.0.0.1:43123/media/${index.toString(16).padStart(64, "0")}`,
    );

    await releaseRecordingPlaybackPaths(playbackPaths, release);

    expect(release).toHaveBeenCalledTimes(30);
    expect(highWaterMark).toBe(maxPlaybackRestoreConcurrency);
  });

  it("trims stale history playback admissions", () => {
    expect(trimHistoryPlaybackAdmissions(
      {
        "keep.txt": { byteLength: 4, playbackPath: "keep-url", sourcePath: "keep.wav" },
        "stale.txt": { byteLength: 8, playbackPath: "stale-url", sourcePath: "stale.wav" },
      },
      [{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "keep",
        outputPath: "keep.txt",
        sourcePath: "keep.wav",
      }],
    )).toEqual({
      "keep.txt": { byteLength: 4, playbackPath: "keep-url", sourcePath: "keep.wav" },
    });
  });

  it("drops a history admission when the same output belongs to a different recording", () => {
    const current = {
      "meeting.txt": {
        byteLength: 4,
        playbackPath: "first-playback-url",
        sourcePath: "first.wav",
      },
    };

    expect(trimHistoryPlaybackAdmissions(current, [{
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "replacement",
      outputPath: "meeting.txt",
      sourcePath: "replacement.wav",
    }])).toEqual({});
  });

  it("projects history playback only when output and source identities both match", () => {
    const admission = {
      byteLength: 4,
      playbackPath: "first-playback-url",
      sourcePath: "first.wav",
    };
    const replacement = {
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "replacement",
      outputPath: "meeting.txt",
      sourcePath: "replacement.wav",
    };

    expect(projectHistoryPlaybackAdmission(replacement, {
      "meeting.txt": admission,
    })).toBeUndefined();
    expect(projectHistoryPlaybackAdmission(
      { ...replacement, sourcePath: "first.wav" },
      { "meeting.txt": admission },
    )).toBe(admission);
  });

  it("deduplicates queue and history restores for the same source path", async () => {
    const restore = vi.fn(async () => ({
      byteLength: 4,
      playbackPath: `http://127.0.0.1:43123/media/${"9".repeat(64)}`,
    }));
    const sourcePath = "C:/meeting.wav";

    const [queueResult, historyResult] = await Promise.all([
      restoreQueuePlaybackPaths([queuedRecording(1, sourcePath)], { restore, runtime: true }),
      restoreHistoryPlaybackAdmission({
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "meeting",
        outputPath: "C:/meeting.txt",
        sourcePath,
      }, { restore, runtime: true }),
    ]);

    expect(restore).toHaveBeenCalledOnce();
    expect(queueResult[0]?.playbackPath).toBe(historyResult?.playbackPath);
  });

  it("retains the restored recording identity with a history admission", () => {
    expect(mergeHistoryPlaybackAdmissions({}, [{
      byteLength: 4,
      outputPath: "meeting.txt",
      playbackPath: "meeting-playback-url",
      sourcePath: "meeting.wav",
    }])).toEqual({
      "meeting.txt": {
        byteLength: 4,
        playbackPath: "meeting-playback-url",
        sourcePath: "meeting.wav",
      },
    });
  });

  it("merges restored history playback admissions only when metadata changes", () => {
    const current = {
      "meeting.txt": {
        byteLength: 4,
        playbackPath: "meeting-playback-url",
        sourcePath: "meeting.wav",
      },
    };

    expect(mergeHistoryPlaybackAdmissions(current, [])).toBe(current);
    expect(mergeHistoryPlaybackAdmissions(current, [
      {
        byteLength: 4,
        outputPath: "meeting.txt",
        playbackPath: "meeting-playback-url",
        sourcePath: "meeting.wav",
      },
    ])).toBe(current);
    expect(mergeHistoryPlaybackAdmissions(current, [
      {
        byteLength: 8,
        outputPath: "other.txt",
        playbackPath: "other-playback-url",
        sourcePath: "other.wav",
      },
    ])).toEqual({
      "meeting.txt": {
        byteLength: 4,
        playbackPath: "meeting-playback-url",
        sourcePath: "meeting.wav",
      },
      "other.txt": {
        byteLength: 8,
        playbackPath: "other-playback-url",
        sourcePath: "other.wav",
      },
    });
  });
});

describe("playback admission lifecycle", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("revokes a removed source immediately and an unclaimed admission after grace", async () => {
    vi.useFakeTimers();
    const revoke = vi.fn(async () => undefined);
    const tracker = createPlaybackAdmissionTracker(revoke, 1_000);
    const first = `http://127.0.0.1:43123/media/${"1".repeat(64)}`;
    const second = `http://127.0.0.1:43123/media/${"2".repeat(64)}`;
    const unclaimed = `http://127.0.0.1:43123/media/${"3".repeat(64)}`;

    tracker.track(first);
    tracker.reconcile([first]);
    tracker.track(second);
    tracker.reconcile([second]);
    expect(revoke).toHaveBeenCalledWith(first);

    tracker.track(unclaimed);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(revoke).toHaveBeenCalledWith(unclaimed);
    tracker.dispose();
  });

  it("keeps failed revocations tracked and retries until native release succeeds", async () => {
    vi.useFakeTimers();
    const revoke = vi.fn()
      .mockRejectedValueOnce(new Error("native release failed"))
      .mockResolvedValue(undefined);
    const tracker = createPlaybackAdmissionTracker(revoke, 1_000);
    const playbackPath = `http://127.0.0.1:43123/media/${"4".repeat(64)}`;

    tracker.track(playbackPath);
    tracker.reconcile([playbackPath]);
    tracker.reconcile([]);
    await Promise.resolve();
    expect(revoke).toHaveBeenCalledOnce();

    await vi.advanceTimersByTimeAsync(1_000);
    expect(revoke).toHaveBeenCalledTimes(2);
    tracker.dispose();
  });
});
