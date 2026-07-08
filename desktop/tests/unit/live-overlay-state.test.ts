import { describe, expect, it } from "vitest";

import type { LiveSessionView } from "@/lib/app-types";

import {
  hoverSensorHeight,
  idleSensorWidth,
  modelFromLiveView,
  overlayFrame,
  overlaySurface,
  peekHeight,
} from "@/components/live/live-overlay-state";

const baseView: LiveSessionView = {
  captureMode: "pushToTalk",
  hotkey: "Control+Win",
  route: "localFallback",
  status: "idle",
  visibility: "enabled",
};

describe("live overlay state projection", () => {
  it("keeps idle invisible until hover opens the peek island", () => {
    const model = modelFromLiveView(baseView);

    expect(overlaySurface(model, false, false, false)).toBe("sensor");
    expect(overlayFrame("sensor", model)).toEqual({ height: hoverSensorHeight, width: idleSensorWidth });
    expect(overlaySurface(model, true, false, false)).toBe("peek");
    expect(overlayFrame("peek", model)).toEqual({ height: peekHeight, width: idleSensorWidth });
  });

  it("treats armed, listening, and speaking as active recording surfaces", () => {
    for (const status of ["armed", "listening", "speaking"] as const) {
      expect(modelFromLiveView({ ...baseView, status }).phase).toBe("recording");
    }
  });

  it("does not let hidden idle preference suppress an active recording", () => {
    const model = modelFromLiveView({ ...baseView, status: "listening", visibility: "hidden" });

    expect(model.phase).toBe("recording");
    expect(overlaySurface(model, false, false, false)).toBe("recording");
  });

  it("reserves the hands-free confirm/cancel island width", () => {
    const model = modelFromLiveView({ ...baseView, captureMode: "toggle", status: "listening" });

    expect(model.recordingTriggerMode).toBe("toggle");
    expect(overlayFrame("recording", model)).toEqual({ height: 40, width: 112 });
  });

  it("uses the active gesture mode over the saved setting", () => {
    const held = modelFromLiveView({
      ...baseView,
      activeCaptureMode: "pushToTalk",
      captureMode: "toggle",
      status: "speaking",
    });
    const handsFree = modelFromLiveView({
      ...baseView,
      activeCaptureMode: "toggle",
      captureMode: "pushToTalk",
      status: "speaking",
    });

    expect(held.recordingTriggerMode).toBe("hold");
    expect(overlayFrame("recording", held)).toEqual({ height: 40, width: 112 });
    expect(handsFree.recordingTriggerMode).toBe("toggle");
    expect(overlayFrame("recording", handsFree)).toEqual({ height: 40, width: 112 });
  });

  it("keeps settling and saving in the compact processing surface", () => {
    for (const status of ["settling", "saving"] as const) {
      const model = modelFromLiveView({ ...baseView, status });

      expect(model.phase).toBe("processing");
      expect(overlayFrame("processing", model).height).toBe(40);
    }
  });

  it("derives success and failure affordance surfaces from current state", () => {
    const idleWithText = modelFromLiveView({ ...baseView, finalText: "done" });
    const blocked = modelFromLiveView({ ...baseView, error: "Mic denied", route: "blocked", status: "blocked" });

    expect(overlaySurface(idleWithText, false, false, true)).toBe("success");
    expect(overlaySurface(blocked, false, false, false)).toBe("feedback");
    expect(overlayFrame("feedback", blocked).width).toBeGreaterThanOrEqual(180);
  });
});
