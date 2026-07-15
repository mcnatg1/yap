import { describe, expect, it } from "vitest";

import {
  MICROPHONE_PERMISSION_DENIED_PREFIX,
  classifyNativeReadiness,
} from "../wdio/task-8b-helpers.js";


describe("Task 8b native readiness classification", () => {
  const ready = {
    deviceError: null,
    devices: [{ id: "0:Microphone" }],
    model: { status: "ready" },
    preflight: { error: null, status: "idle" },
  };

  it.each(["missing", "disabled", "corrupted"])("skips only the precise %s model state", (status) => {
    expect(classifyNativeReadiness({ ...ready, model: { status } })).toMatchObject({
      action: "skip",
    });
  });

  it("skips a successful enumeration that returns zero devices", () => {
    expect(classifyNativeReadiness({ ...ready, devices: [], preflight: null })).toEqual({
      action: "skip",
      reason: "no input device was enumerated",
    });
  });

  it("skips only a native error carrying the narrow permission marker", () => {
    const permissionError = `${MICROPHONE_PERMISSION_DENIED_PREFIX} Access is denied. (0x80070005)`;
    expect(classifyNativeReadiness({ ...ready, deviceError: permissionError, devices: null })).toMatchObject({
      action: "skip",
    });
    expect(classifyNativeReadiness({
      ...ready,
      preflight: { error: permissionError, status: "blocked" },
    })).toMatchObject({ action: "skip" });
  });

  it.each([
    "Microphone device enumeration failed: backend unavailable",
    "Microphone default input configuration failed: unsupported format",
    "Microphone input stream build failed: backend regression",
    "Microphone input stream playback failed: device invalidated",
    "Microphone access failed: generic legacy prefix",
    "Permission denied",
  ])("fails instead of skipping native regression: %s", (error) => {
    expect(() => classifyNativeReadiness({ ...ready, deviceError: error, devices: null }))
      .toThrow(/enumeration failed/i);
    expect(() => classifyNativeReadiness({
      ...ready,
      preflight: { error, status: "blocked" },
    })).toThrow(/preflight failure/i);
  });

  it("fails unknown blocked and no-sample preflights", () => {
    expect(() => classifyNativeReadiness({
      ...ready,
      preflight: { error: "No input detected.", status: "blocked" },
    })).toThrow(/preflight failure/i);
    expect(() => classifyNativeReadiness({
      ...ready,
      preflight: { error: null, status: "blocked" },
    })).toThrow(/unknown error/i);
  });

  it.each(["missing", "disabled", "corrupted"])(
    "does not let the %s model skip hide a simultaneous native regression",
    (status) => {
      expect(() => classifyNativeReadiness({
        ...ready,
        deviceError: "Microphone device enumeration failed: backend unavailable",
        devices: null,
        model: { status },
      })).toThrow(/enumeration failed/i);
      expect(() => classifyNativeReadiness({
        ...ready,
        model: { status },
        preflight: {
          error: "Microphone input stream build failed: backend regression",
          status: "blocked",
        },
      })).toThrow(/preflight failure/i);
    },
  );
});
