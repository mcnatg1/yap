import { describe, expect, it } from "vitest";

import {
  liveRouteLabel,
  liveStatusLabel,
  type LiveSessionView,
} from "@/lib/live-session";

describe("live session projection", () => {
  it("labels all routes", () => {
    expect(liveRouteLabel("none")).toBe("Idle");
    expect(liveRouteLabel("localFallback")).toBe("Local fallback");
    expect(liveRouteLabel("serverLive")).toBe("Server");
    expect(liveRouteLabel("blocked")).toBe("Needs setup");
  });

  it("labels blocked status without motion state", () => {
    expect(liveStatusLabel("blocked")).toBe("Blocked");

    const view: LiveSessionView = {
      captureMode: "pushToTalk",
      error: "Mic denied",
      hotkey: "Ctrl+Shift+Space",
      pasteHotkey: "",
      route: "blocked",
      status: "blocked",
      visibility: "enabled",
    };

    expect(view.error).toBe("Mic denied");
    expect(view.route).toBe("blocked");
  });

  it("can expose a copyable final text after live returns to idle", () => {
    const view: LiveSessionView = {
      captureMode: "toggle",
      finalText: "hello world",
      hotkey: "Ctrl+Win",
      pasteHotkey: "",
      route: "none",
      status: "idle",
      visibility: "enabled",
    };

    expect(view.status).toBe("idle");
    expect(view.finalText).toBe("hello world");
  });
});
