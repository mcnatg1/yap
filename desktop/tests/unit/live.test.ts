import { beforeEach, describe, expect, it, vi } from "vitest";

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => tauri);
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

import { resolveOwnedLiveTranscriptPaths } from "@/live";

describe("live native bridge", () => {
  beforeEach(() => {
    tauri.invoke.mockReset();
    tauri.isTauri.mockReset();
    tauri.isTauri.mockReturnValue(true);
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
});
