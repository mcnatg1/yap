import { describe, expect, it } from "vitest";

import {
  liveRouteLabel,
  liveStatusLabel,
  type LiveSessionView,
} from "@/lib/app-types";

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
      route: "blocked",
      status: "blocked",
      visibility: "enabled",
    };

    expect(view.error).toBe("Mic denied");
    expect(view.route).toBe("blocked");
  });
});
