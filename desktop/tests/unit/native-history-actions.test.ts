import { describe, expect, it } from "vitest";

import { readTranscriptHistory } from "@/history-storage";
import {
  canDeleteTranscriptHistoryEntry,
  historyEntryPlaybackPath,
  recoverableLiveSessionActionIdentity,
  savedLiveSessionActionIdentity,
  savedSessionToTranscriptHistoryEntry,
} from "@/native-history";

describe("native transcript history actions", () => {
  it("projects saved live sessions into history entries", () => {
    const entry = savedSessionToTranscriptHistoryEntry({
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-1",
      outputPath: "live-1.txt",
      sessionId: "1",
      sourcePath: "live-1.wav",
      warning: null,
    });

    expect(entry).toEqual({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-1",
      outputPath: "live-1.txt",
      sessionId: "1",
      sourcePath: "live-1.wav",
      warning: undefined,
    });
  });

  it("preserves old localStorage rows while retaining native recovery metadata", () => {
    const storage = {
      getItem: () => JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "legacy-live",
        outputPath: "legacy-live.txt",
        sourcePath: "legacy-live.wav",
      }]),
      setItem: () => undefined,
    };

    expect(readTranscriptHistory(storage)).toEqual([{
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "legacy-live",
      outputPath: "legacy-live.txt",
      sourcePath: "legacy-live.wav",
    }]);

    const entry = savedSessionToTranscriptHistoryEntry({
      createdAtMs: Date.UTC(2026, 0, 2),
      name: "live-recoverable",
      outputPath: "live-recoverable.wav.part",
      sessionId: "recoverable",
      sourcePath: "live-recoverable.wav.part",
      warning: null,
      captureCommitPath: undefined,
      recoveryState: "recoverable",
    });

    expect(entry.recoveryState).toBe("recoverable");
    expect(entry.captureCommitPath).toBeUndefined();
  });

  it("only exposes delete for Yap-owned live history entries", () => {
    expect(canDeleteTranscriptHistoryEntry(savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.commit.json",
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sessionId: "123",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav",
    }))).toBe(true);

    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "meeting-notes",
      outputPath: "C:\\Users\\me\\Documents\\meeting-notes.txt",
      sourcePath: "C:\\Users\\me\\Downloads\\meeting.mp3",
    })).toBe(false);

    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\Documents\\live-123.txt",
      sourcePath: "C:\\Users\\me\\Documents\\live-123.wav",
    })).toBe(false);

    expect(canDeleteTranscriptHistoryEntry({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sessionId: "123",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-999.wav",
    })).toBe(false);

    expect(canDeleteTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.commit.json",
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-999",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt",
      sessionId: "999",
      sourcePath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav",
    })).toBe(false);
  });

  it("only exposes playback for matching Yap-owned live audio", () => {
    const outputPath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.txt";
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.wav";

    expect(historyEntryPlaybackPath(savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-123.commit.json",
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-123",
      outputPath,
      sessionId: "123",
      sourcePath,
    }))).toBe(sourcePath);

    expect(historyEntryPlaybackPath({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-123",
      outputPath,
      sourcePath: outputPath,
    })).toBeUndefined();

    expect(historyEntryPlaybackPath({
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "meeting-notes",
      outputPath: "C:\\Users\\me\\Documents\\meeting-notes.txt",
      sourcePath: "C:\\Users\\me\\Downloads\\meeting.mp3",
    })).toBeUndefined();
  });

  it("keeps a committed audio-only session actionable through its WAV identity", () => {
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-audio.wav";
    const entry = savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-audio.commit.json",
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-audio",
      outputPath: sourcePath,
      sessionId: "audio",
      sourcePath,
    });

    expect(savedLiveSessionActionIdentity(entry)).toEqual({
      expectedCaptureCommitPath: entry.captureCommitPath,
      expectedOutputPath: sourcePath,
      sessionId: "audio",
    });
    expect(canDeleteTranscriptHistoryEntry(entry)).toBe(true);
    expect(historyEntryPlaybackPath(entry)).toBe(sourcePath);
  });

  it("uses a recovered partial row's source WAV as its native action identity", () => {
    const sourcePath = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.wav";
    const entry = savedSessionToTranscriptHistoryEntry({
      captureCommitPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.commit.json",
      createdAtMs: Date.UTC(2026, 0, 1),
      name: "live-recovered",
      outputPath: "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-recovered.txt",
      recoveryState: "recovered",
      sessionId: "recovered",
      sourcePath,
    });

    expect(recoverableLiveSessionActionIdentity(entry)).toEqual({
      expectedArtifactPath: sourcePath,
      sessionId: "recovered",
    });
    expect(savedLiveSessionActionIdentity(entry)).toBeUndefined();
  });
});
