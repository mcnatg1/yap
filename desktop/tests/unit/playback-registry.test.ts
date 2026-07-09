import { describe, expect, it } from "vitest";

import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";
import {
  applyRestoredQueuePlaybackPaths,
  mergeHistoryPlaybackPaths,
  trimHistoryPlaybackPaths,
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
      [{ id: 1, playbackPath: "\\\\?\\C:\\meeting.wav" }],
    );

    expect(next[0]).toMatchObject({ playbackPath: "\\\\?\\C:\\meeting.wav" });
    expect(next[1]).toBe(second);
  });

  it("trims stale history playback paths", () => {
    expect(trimHistoryPlaybackPaths(
      { "keep.txt": "keep.wav", "stale.txt": "stale.wav" },
      [{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "keep",
        outputPath: "keep.txt",
        sourcePath: "keep.wav",
      }],
    )).toEqual({ "keep.txt": "keep.wav" });
  });

  it("merges restored history playback paths only when values change", () => {
    const current = { "meeting.txt": "meeting.wav" };

    expect(mergeHistoryPlaybackPaths(current, [])).toBe(current);
    expect(mergeHistoryPlaybackPaths(current, [
      { outputPath: "meeting.txt", playbackPath: "meeting.wav" },
    ])).toBe(current);
    expect(mergeHistoryPlaybackPaths(current, [
      { outputPath: "other.txt", playbackPath: "other.wav" },
    ])).toEqual({ "meeting.txt": "meeting.wav", "other.txt": "other.wav" });
  });
});
