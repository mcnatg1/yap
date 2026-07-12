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
  releaseRecordingPlaybackPaths,
  restoreQueuePlaybackPaths,
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
        "keep.txt": { byteLength: 4, playbackPath: "keep.wav" },
        "stale.txt": { byteLength: 8, playbackPath: "stale.wav" },
      },
      [{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "keep",
        outputPath: "keep.txt",
        sourcePath: "keep.wav",
      }],
    )).toEqual({ "keep.txt": { byteLength: 4, playbackPath: "keep.wav" } });
  });

  it("merges restored history playback admissions only when metadata changes", () => {
    const current = {
      "meeting.txt": { byteLength: 4, playbackPath: "meeting.wav" },
    };

    expect(mergeHistoryPlaybackAdmissions(current, [])).toBe(current);
    expect(mergeHistoryPlaybackAdmissions(current, [
      { byteLength: 4, outputPath: "meeting.txt", playbackPath: "meeting.wav" },
    ])).toBe(current);
    expect(mergeHistoryPlaybackAdmissions(current, [
      { byteLength: 8, outputPath: "other.txt", playbackPath: "other.wav" },
    ])).toEqual({
      "meeting.txt": { byteLength: 4, playbackPath: "meeting.wav" },
      "other.txt": { byteLength: 8, playbackPath: "other.wav" },
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
});
