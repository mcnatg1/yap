import { describe, expect, it } from "vitest";

import type { LiveSessionView } from "@/lib/app-types";

import {
  collapseGraceMs,
  modelFromLiveView,
  overlaySurface,
  previewOverlayFrame,
} from "@/components/live/live-overlay-state";

const baseView: LiveSessionView = {
  captureMode: "pushToTalk",
  hotkey: "Control+Win",
  pasteHotkey: "",
  route: "localFallback",
  status: "idle",
  visibility: "enabled",
};

describe("live overlay state projection", () => {
  it("keeps a visible collapsed island and expands the same surface downward", () => {
    const model = modelFromLiveView(baseView);

    expect(overlaySurface(model, false, false)).toBe("collapsed");
    expect(previewOverlayFrame("collapsed")).toEqual({ height: 40, width: 104 });
    expect(overlaySurface(model, true, false)).toBe("expanded");
    expect(previewOverlayFrame("expanded")).toEqual({ height: 88, width: 180 });
    expect(collapseGraceMs).toBe(200);
  });

  it("shows armed as initializing before capture is installed", () => {
    expect(modelFromLiveView({ ...baseView, status: "armed" }).phase).toBe("initializing");
  });

  it("treats listening and speaking as active recording surfaces", () => {
    for (const status of ["listening", "speaking"] as const) {
      expect(modelFromLiveView({ ...baseView, status }).phase).toBe("recording");
    }
  });

  it("does not let hidden idle preference suppress an active recording", () => {
    const model = modelFromLiveView({ ...baseView, status: "listening", visibility: "hidden" });

    expect(model.phase).toBe("recording");
    expect(overlaySurface(model, false, false)).toBe("recording");
  });

  it("reserves the hands-free finish island width", () => {
    const model = modelFromLiveView({ ...baseView, captureMode: "toggle", status: "listening" });

    expect(model.recordingTriggerMode).toBe("toggle");
    expect(previewOverlayFrame("recording")).toEqual({ height: 40, width: 112 });
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
    expect(previewOverlayFrame("recording")).toEqual({ height: 40, width: 112 });
    expect(handsFree.recordingTriggerMode).toBe("toggle");
    expect(previewOverlayFrame("recording")).toEqual({ height: 40, width: 112 });
  });

  it("keeps settling and saving in the compact processing surface", () => {
    for (const status of ["settling", "saving"] as const) {
      const model = modelFromLiveView({ ...baseView, status });

      expect(model.phase).toBe("processing");
      expect(previewOverlayFrame("processing")).toEqual({ height: 40, width: 112 });
    }
  });

  it("derives success and failure affordance surfaces from current state", () => {
    const idleWithText = modelFromLiveView({ ...baseView, finalText: "done" });
    const blocked = modelFromLiveView({ ...baseView, error: "Mic denied", route: "blocked", status: "blocked" });

    expect(overlaySurface(idleWithText, false, true)).toBe("success");
    expect(previewOverlayFrame("success")).toEqual({ height: 40, width: 168 });
    expect(overlaySurface(blocked, false, false)).toBe("feedback");
    expect(previewOverlayFrame("feedback")).toEqual({ height: 40, width: 252 });
  });

  it("surfaces idle injection fallback instead of reporting success", () => {
    const fallback = modelFromLiveView({
      ...baseView,
      error: "Couldn't insert text here. Transcript copied; press Ctrl+V.",
      finalText: "done",
    });

    expect(fallback.phase).toBe("feedback");
    expect(fallback.errorMessage).toContain("Transcript copied");
    expect(overlaySurface(fallback, false, true)).toBe("feedback");
  });
});
