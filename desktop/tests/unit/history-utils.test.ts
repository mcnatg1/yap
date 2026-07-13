import { describe, expect, it } from "vitest";

import { historyEntryToRecordingJob } from "@/lib/history-utils";
import {
  canDeleteTranscriptHistoryEntry,
  historyEntryPlaybackPath,
  savedSessionToTranscriptHistoryEntry,
} from "@/history";

describe("history job projection", () => {
  it("projects partial live saves with their warning", () => {
    const job = historyEntryToRecordingJob({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      warning: "Live audio could not be saved. Transcript was saved.",
    });

    expect(job.status).toBe("partial");
    expect(job.id).toBe("history:c:\\users\\me\\appdata\\local\\yap\\live-recordings\\live-123.txt");
    expect(job.sessionOrigin).toBe("liveCapture");
    expect(job.playbackPath).toBeUndefined();
    expect(job.pipeline.postprocessing).toBe("error");
  });

  it("does not expose owned live audio until playback is admitted", () => {
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav";
    const job = historyEntryToRecordingJob(savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.commit.json",
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sessionId: "123",
      sourcePath,
    }));

    expect(job.sessionOrigin).toBe("liveCapture");
    expect(job.sourcePath).toBe(sourcePath);
    expect(job.playbackPath).toBeUndefined();
  });

  it("does not trust foreign history source paths for playback", () => {
    const job = historyEntryToRecordingJob({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "meeting",
      outputPath: "C:\\Users\\me\\Documents\\meeting.txt",
      sourcePath: "C:\\Users\\me\\Downloads\\meeting.wav",
    });

    expect(job.id).toBe("history:c:\\users\\me\\documents\\meeting.txt");
    expect(job.playbackPath).toBeUndefined();
  });

  it("accepts a native-restored playback path for registered imports", () => {
    const restoredPath = "\\\\?\\C:\\Users\\me\\Downloads\\meeting.wav";
    const job = historyEntryToRecordingJob(
      {
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "meeting",
        outputPath: "C:\\Users\\me\\Documents\\meeting.txt",
        sourcePath: "C:\\Users\\me\\Downloads\\meeting.wav",
      },
      restoredPath,
    );

    expect(job.playbackPath).toBe(restoredPath);
  });

  it("keeps every partial recovery state out of normal history actions", () => {
    const job = historyEntryToRecordingJob({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-recoverable",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recoverable.wav.part",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recoverable.wav.part",
      recoveryState: "recoverable",
    } as Parameters<typeof historyEntryToRecordingJob>[0] & {
      recoveryState?: "recoverable";
    });

    expect(job.status).toBe("partial");
    expect(job.playbackPath).toBeUndefined();
    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-recoverable",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recoverable.wav.part",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recoverable.wav.part",
      recoveryState: "recoverable",
    })).toBe(false);
    expect(historyEntryPlaybackPath({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-recoverable",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recoverable.wav.part",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recoverable.wav.part",
      recoveryState: "recoverable",
    })).toBeUndefined();

    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-recovered",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.wav",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.wav",
      recoveryState: "recovered",
    })).toBe(false);
    expect(historyEntryPlaybackPath({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-recovered",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.wav",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.wav",
      recoveryState: "recovered",
    })).toBeUndefined();
  });
});
