import { describe, expect, it } from "vitest";

import { createInitialPipelineState, type RecordingJobView } from "@/lib/app-types";
import {
  applyRestoredQueuePlaybackPaths,
  mergeHistoryPlaybackAdmissions,
  trimHistoryPlaybackAdmissions,
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
