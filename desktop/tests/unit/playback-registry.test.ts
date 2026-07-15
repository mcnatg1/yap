import { afterEach, describe, expect, it, vi } from "vitest";

import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";
import {
  currentPlaybackPaths,
  mergeHistoryPlaybackAdmissions,
  projectHistoryPlaybackAdmission,
  restoreHistoryPlaybackAdmission,
  trimHistoryPlaybackAdmissions,
} from "@/lib/history-playback";
import {
  createPlaybackAdmissionTracker,
  releaseRecordingPlaybackPaths,
  validatePlaybackAdmission,
} from "@/lib/playback-admission";
import {
  maxPlaybackRestoreConcurrency,
  playbackAdmissionDeadlineMs,
  settlePlaybackAdmissionBeforeDeadline,
} from "@/lib/playback-admission-queue";

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

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((accept) => {
    resolve = accept;
  });
  return { promise, resolve };
}

async function flushMicrotasks() {
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
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

  it("claims a restored history admission until React reconciles active playback", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-15T12:00:00.000Z"));
    const claim = vi.fn();
    const admittedPath = playbackPath("7");

    await restoreHistoryPlaybackAdmission({
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "meeting",
      outputPath: "meeting.txt",
      sourcePath: "meeting.wav",
    }, {
      claim,
      restore: async () => ({ byteLength: 4, playbackPath: admittedPath }),
      runtime: true,
    });

    expect(claim).toHaveBeenCalledWith(
      admittedPath,
      Date.now() + playbackAdmissionDeadlineMs,
    );
  });

  it("coalesces concurrent restores for the same source without conflating consumers", async () => {
    const gate = deferred<{ byteLength: number; playbackPath: string }>();
    const restore = vi.fn(() => gate.promise);
    const first = restoreHistoryPlaybackAdmission({
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "first",
      outputPath: "first.txt",
      sourcePath: "shared.wav",
    }, { restore, runtime: true });
    const second = restoreHistoryPlaybackAdmission({
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "second",
      outputPath: "second.txt",
      sourcePath: "shared.wav",
    }, { restore, runtime: true });

    expect(restore).toHaveBeenCalledOnce();
    gate.resolve({ byteLength: 4, playbackPath: playbackPath("c") });

    await expect(Promise.all([first, second])).resolves.toEqual([
      expect.objectContaining({ outputPath: "first.txt", sourcePath: "shared.wav" }),
      expect.objectContaining({ outputPath: "second.txt", sourcePath: "shared.wav" }),
    ]);
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

  it("releases admission scheduler capacity before waiting for late cleanup", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-15T12:00:00.000Z"));
    const gates = Array.from(
      { length: maxPlaybackRestoreConcurrency },
      () => deferred<{ byteLength: number; playbackPath: string }>(),
    );
    const neverRelease = vi.fn(() => new Promise<never>(() => undefined));
    const late = gates.map((gate) => settlePlaybackAdmissionBeforeDeadline(
      () => gate.promise,
      neverRelease,
      Date.now() + 100,
    ));
    const nextAdmission = { byteLength: 1, playbackPath: playbackPath("e") };
    const admitNext = vi.fn(async () => nextAdmission);
    const next = settlePlaybackAdmissionBeforeDeadline(
      admitNext,
      async () => undefined,
      Date.now() + 1_000,
    );

    expect(admitNext).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(100);
    await expect(Promise.all(late)).resolves.toEqual(
      Array.from({ length: maxPlaybackRestoreConcurrency }, () => undefined),
    );
    gates.forEach((gate, index) => gate.resolve({
      byteLength: 1,
      playbackPath: playbackPath((index + 8).toString(16)),
    }));
    await flushMicrotasks();

    expect(neverRelease).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);
    expect(admitNext).toHaveBeenCalledOnce();
    await expect(next).resolves.toEqual(nextAdmission);
  });

  it("releases history scheduler capacity before waiting for abandoned cleanup", async () => {
    const gates = Array.from(
      { length: maxPlaybackRestoreConcurrency },
      () => deferred<{ byteLength: number; playbackPath: string }>(),
    );
    const controllers = gates.map(() => new AbortController());
    const neverRelease = vi.fn(() => new Promise<never>(() => undefined));
    const abandoned = gates.map((gate, index) => restoreHistoryPlaybackAdmission({
      createdAt: "2026-01-02T00:00:00.000Z",
      name: `meeting-${index}`,
      outputPath: `meeting-${index}.txt`,
      sourcePath: `meeting-${index}.wav`,
    }, {
      release: neverRelease,
      restore: () => gate.promise,
      runtime: true,
      signal: controllers[index].signal,
    }));
    const restoreNext = vi.fn(async () => ({
      byteLength: 1,
      playbackPath: playbackPath("f"),
    }));
    const next = restoreHistoryPlaybackAdmission({
      createdAt: "2026-01-02T00:00:00.000Z",
      name: "next",
      outputPath: "next.txt",
      sourcePath: "next.wav",
    }, {
      restore: restoreNext,
      runtime: true,
    });

    expect(restoreNext).not.toHaveBeenCalled();
    controllers.forEach((controller) => controller.abort());
    gates.forEach((gate, index) => gate.resolve({
      byteLength: 1,
      playbackPath: playbackPath((index + 1).toString(16)),
    }));
    await flushMicrotasks();

    expect(neverRelease).toHaveBeenCalledTimes(maxPlaybackRestoreConcurrency);
    expect(restoreNext).toHaveBeenCalledOnce();
    await expect(Promise.all(abandoned)).resolves.toEqual(
      Array.from({ length: maxPlaybackRestoreConcurrency }, () => undefined),
    );
    await expect(next).resolves.toMatchObject({ outputPath: "next.txt" });
  });
});
