import { beforeEach, describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => tauri);
vi.mock("@tauri-apps/api/event", () => ({ listen: tauri.listen }));

import {
  deleteRecoverableLiveSession,
  deleteSavedLiveSession,
  listenLiveOverlaySession,
  listenLiveSessionSaved,
  liveOverlayStatus,
  recoverLiveSession,
  resolveOwnedLiveTranscriptPaths,
  startLiveOverlaySession,
  stopLiveOverlaySession,
} from "@/live";

describe("live native bridge", () => {
  beforeEach(() => {
    tauri.invoke.mockReset();
    tauri.isTauri.mockReset();
    tauri.isTauri.mockReturnValue(true);
    tauri.listen.mockReset();
  });

  it("asks Rust to resolve hidden transcript paths", async () => {
    const resolutions = [{ canonicalPath: null, missing: true, requestedPath: "live-1.txt" }];
    tauri.invoke.mockResolvedValue(resolutions);

    await expect(resolveOwnedLiveTranscriptPaths(["live-1.txt"])).resolves.toEqual(resolutions);
    expect(tauri.invoke).toHaveBeenCalledWith("resolve_owned_live_transcript_paths", {
      outputPaths: ["live-1.txt"],
    });
  });

  it("does not invoke native commands in the browser preview", async () => {
    tauri.isTauri.mockReturnValue(false);

    await expect(resolveOwnedLiveTranscriptPaths(["live-1.txt"])).resolves.toEqual([]);
    expect(tauri.invoke).not.toHaveBeenCalled();
  });

  it("binds history mutations to the expected native artifact identity", async () => {
    tauri.invoke.mockResolvedValue(undefined);

    await deleteSavedLiveSession("123", "C:/Yap/live-123.txt", "C:/Yap/live-123.commit.json");
    await deleteRecoverableLiveSession("partial", "C:/Yap/live-partial.wav.part");
    await recoverLiveSession("partial", "C:/Yap/live-partial.wav.part");

    expect(tauri.invoke.mock.calls).toEqual([
      ["delete_saved_live_session", {
        expectedCaptureCommitPath: "C:/Yap/live-123.commit.json",
        expectedOutputPath: "C:/Yap/live-123.txt",
        sessionId: "123",
      }],
      ["delete_recoverable_live_session", {
        expectedArtifactPath: "C:/Yap/live-partial.wav.part",
        sessionId: "partial",
      }],
      ["recover_live_session", {
        expectedArtifactPath: "C:/Yap/live-partial.wav.part",
        sessionId: "partial",
      }],
    ]);
  });

  it("forwards saved-session payloads and returns the native unlistener", async () => {
    const stop = vi.fn();
    const onSaved = vi.fn();
    const saved = {
      createdAtMs: 123,
      name: "live-123",
      outputPath: "C:/Yap/live-123.txt",
      sessionId: "123",
      sourcePath: "C:/Yap/live-123.wav",
    };
    tauri.listen.mockResolvedValue(stop);

    const unlisten = await listenLiveSessionSaved(onSaved);
    const handler = tauri.listen.mock.calls[0]?.[1];
    handler?.({ payload: saved });

    expect(tauri.listen.mock.calls[0]?.[0]).toBe("live-session-saved");
    expect(onSaved).toHaveBeenCalledWith(saved);
    expect(unlisten).toBe(stop);
  });

  it("returns a no-op saved-session listener outside Tauri", async () => {
    tauri.isTauri.mockReturnValue(false);

    const unlisten = await listenLiveSessionSaved(vi.fn());

    expect(tauri.listen).not.toHaveBeenCalled();
    expect(unlisten()).toBeUndefined();
  });

  it("uses the minimized overlay command and event contracts", async () => {
    const stop = vi.fn();
    const onUpdate = vi.fn();
    const overlay = {
      captureMode: "pushToTalk",
      hasFinalText: false,
      status: "idle",
      visibility: "enabled",
    };
    tauri.invoke.mockResolvedValue(overlay);
    tauri.listen.mockResolvedValue(stop);

    await expect(liveOverlayStatus()).resolves.toEqual(overlay);
    await expect(startLiveOverlaySession("toggle")).resolves.toEqual(overlay);
    await expect(stopLiveOverlaySession()).resolves.toEqual(overlay);
    const unlisten = await listenLiveOverlaySession(onUpdate);
    const handler = tauri.listen.mock.calls[0]?.[1];
    handler?.({ payload: overlay });

    expect(tauri.invoke.mock.calls).toEqual([
      ["live_overlay_status"],
      ["start_live_overlay_session", { activeCaptureMode: "toggle" }],
      ["stop_live_overlay_session"],
    ]);
    expect(tauri.listen.mock.calls[0]?.[0]).toBe("live-overlay-session");
    expect(onUpdate).toHaveBeenCalledWith();
    expect(unlisten).toBe(stop);
  });
});
