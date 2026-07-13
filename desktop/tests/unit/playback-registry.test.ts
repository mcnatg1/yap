import { afterEach, describe, expect, it, vi } from "vitest";

import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";
import {
  createPlaybackAdmissionTracker,
  currentPlaybackPaths,
  mergeHistoryPlaybackAdmissions,
  projectHistoryPlaybackAdmission,
  releaseRecordingPlaybackPaths,
  restoreHistoryPlaybackAdmission,
  trimHistoryPlaybackAdmissions,
  validatePlaybackAdmission,
} from "@/lib/playback-registry";

function playbackPath(token: string) {
  return `http://127.0.0.1:43123/media/${token.repeat(64).slice(0, 64)}`;
}

function queuedRecording(id: string, sourcePath: string): RecordingJobView {
  return {
    id,
    name: sourcePath.split("/").pop() ?? sourcePath,
    pipeline: createInitialPipelineState(),
    route: "serverBatch",
    sessionMode: "meeting",
    sessionOrigin: "importedFile",
    sourcePath,
    status: "queued_server",
  };
}

describe("playback registry projections", () => {
  afterEach(() => vi.useRealTimers());

  it("accepts only loopback native admissions and preserves the waveform eligibility bound", () => {
    expect(validatePlaybackAdmission({
      byteLength: "42",
      playbackPath: playbackPath("a"),
      waveformEligible: true,
    })).toEqual({ byteLength: 0, playbackPath: playbackPath("a") });
    expect(() => validatePlaybackAdmission({
      byteLength: "42",
      playbackPath: "file:///C:/meeting.wav",
      waveformEligible: true,
    })).toThrow("Invalid playback admission");
  });

  it("uses Rust-projected queue playback paths without restoring or mutating jobs", () => {
    const active = { ...queuedRecording("job-active", "C:/active.wav"), playbackPath: playbackPath("1") };
    const failed = {
      ...queuedRecording("job-failed", "C:/failed.wav"),
      playbackPath: playbackPath("2"),
      status: "failed" as const,
    };

    expect(currentPlaybackPaths([active, failed], {})).toEqual([active.playbackPath]);
    expect(active).toHaveProperty("sourcePath", "C:/active.wav");
    expect(active).not.toHaveProperty("path");
  });

  it("projects history playback only when output and source identities match", () => {
    const entry = {
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "meeting",
      outputPath: "meeting.txt",
      sourcePath: "meeting.wav",
    };
    const admission = {
      byteLength: 4,
      playbackPath: playbackPath("3"),
      sourcePath: "meeting.wav",
    };

    expect(projectHistoryPlaybackAdmission(entry, { "meeting.txt": admission })).toBe(admission);
    expect(projectHistoryPlaybackAdmission(
      { ...entry, sourcePath: "replacement.wav" },
      { "meeting.txt": admission },
    )).toBeUndefined();
    expect(trimHistoryPlaybackAdmissions({ "meeting.txt": admission }, [entry])).toEqual({
      "meeting.txt": admission,
    });
    expect(mergeHistoryPlaybackAdmissions({}, [{
      ...admission,
      outputPath: "meeting.txt",
    }])).toEqual({ "meeting.txt": admission });
  });

  it("restores only history playback through the native history admission seam", async () => {
    const restore = vi.fn(async () => ({ byteLength: 4, playbackPath: playbackPath("4") }));
    const restored = await restoreHistoryPlaybackAdmission({
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "meeting",
      outputPath: "meeting.txt",
      sourcePath: "meeting.wav",
    }, { restore, runtime: true });

    expect(restore).toHaveBeenCalledWith("meeting.wav");
    expect(restored).toEqual({
      byteLength: 4,
      outputPath: "meeting.txt",
      playbackPath: playbackPath("4"),
      sourcePath: "meeting.wav",
    });
  });

  it("revokes removed and unclaimed admissions", async () => {
    vi.useFakeTimers();
    const revoke = vi.fn(async () => undefined);
    const tracker = createPlaybackAdmissionTracker(revoke, 1_000);
    const first = playbackPath("5");
    const unclaimed = playbackPath("6");

    tracker.track(first);
    tracker.reconcile([first]);
    tracker.reconcile([]);
    tracker.track(unclaimed);
    await vi.advanceTimersByTimeAsync(1_000);

    expect(revoke).toHaveBeenCalledWith(first);
    expect(revoke).toHaveBeenCalledWith(unclaimed);
    tracker.dispose();
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
    const paths = Array.from({ length: 12 }, (_, index) => playbackPath(index.toString(16)));

    await releaseRecordingPlaybackPaths(paths, release);

    expect(release).toHaveBeenCalledTimes(12);
    expect(highWaterMark).toBe(4);
  });
});
