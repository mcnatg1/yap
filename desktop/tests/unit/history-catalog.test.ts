import { describe, expect, it } from "vitest";

import { nativeHistoryIdentity } from "@/history-catalog";
import type { TranscriptHistoryEntry } from "@/history-model";

function entry(overrides: Partial<TranscriptHistoryEntry> = {}): TranscriptHistoryEntry {
  return {
    createdAt: "2026-07-15T12:00:00.000Z",
    name: "remote-1",
    origin: "remote",
    outputPath: "C:/Yap/remote-1.txt",
    sessionId: "remote-1",
    sourcePath: "C:/Yap/remote-1.wav",
    ...overrides,
  };
}

describe("native history catalog identity", () => {
  it("requires native provenance and an opaque session id", () => {
    expect(nativeHistoryIdentity(entry())).toEqual({
      origin: "remote",
      outputPath: "C:/Yap/remote-1.txt",
      sessionId: "remote-1",
    });
    expect(nativeHistoryIdentity(entry({ origin: undefined }))).toBeUndefined();
    expect(nativeHistoryIdentity(entry({ sessionId: undefined }))).toBeUndefined();
  });
});
