import { describe, expect, it } from "vitest";

import {
  createInitialPipelineState,
  deriveSetupState,
  isRecordingActive,
  isRecordingFinished,
  isRecordingRetryable,
  isRecordingRunnable,
  isWorkspaceView,
  fallbackModelLabel,
  recordingStatusForStartFailure,
  serverConnectionLabel,
  setupStateLabel,
} from "./app-types";

describe("client recording workflow projection", () => {
  it("initializes future pipeline stages without running them", () => {
    expect(createInitialPipelineState()).toEqual({
      intake: "queued",
      preprocessing: "notStarted",
      transcription: "notStarted",
      alignment: "notStarted",
      diarization: "notStarted",
      postprocessing: "notStarted",
    });
  });

  it("derives and labels setup state tersely", () => {
    expect(deriveSetupState({ engineReady: true, fallbackEnabled: true, modelInstalled: true })).toBe("fallback_ready");
    expect(deriveSetupState({ engineReady: false, fallbackEnabled: true, modelInstalled: false })).toBe("fallback_missing");
    expect(deriveSetupState({ engineReady: true, fallbackEnabled: false, modelInstalled: true })).toBe("fallback_disabled");

    expect(setupStateLabel("checking")).toBe("Checking");
    expect(setupStateLabel("fallback_missing")).toBe("Setup");
    expect(setupStateLabel("fallback_installing")).toBe("Installing");
    expect(setupStateLabel("fallback_ready")).toBe("Ready");
    expect(setupStateLabel("fallback_disabled")).toBe("Disabled");
    expect(setupStateLabel("setup_error")).toBe("Needs attention");
  });

  it("labels server state tersely", () => {
    expect(serverConnectionLabel("not_set")).toBe("Not set");
    expect(serverConnectionLabel("connecting")).toBe("Checking");
    expect(serverConnectionLabel("ready")).toBe("Ready");
    expect(serverConnectionLabel("offline")).toBe("Offline");
    expect(serverConnectionLabel("sign_in_required")).toBe("Sign in");
    expect(serverConnectionLabel("retrying")).toBe("Retrying");
    expect(serverConnectionLabel("disabled")).toBe("Disabled");
  });

  it("labels the pinned local fallback model clearly", () => {
    expect(fallbackModelLabel("Nemotron 3.5 ASR Streaming 0.6B INT8")).toBe("Nemotron 3.5 ASR Streaming 0.6B INT8");
    expect(fallbackModelLabel("custom.gguf")).toBe("custom");
  });

  it("keeps active, finished, and runnable statuses distinct", () => {
    expect(isRecordingActive("local_transcribing")).toBe(true);
    expect(isRecordingActive("queued_local_fallback")).toBe(false);
    expect(isRecordingFinished("complete")).toBe(true);
    expect(isRecordingFinished("partial")).toBe(true);
    expect(isRecordingRunnable("blocked_server_unavailable")).toBe(false);
    expect(isRecordingRunnable("queued_local_fallback")).toBe(true);
    expect(isRecordingRunnable("failed")).toBe(true);
    expect(isRecordingRunnable("complete")).toBe(false);
    expect(isRecordingRetryable("blocked_server_unavailable")).toBe(false);
    expect(isRecordingRetryable("blocked_sign_in_required")).toBe(true);
    expect(isRecordingRetryable("blocked_setup_required")).toBe(false);
  });

  it("maps rejected starts into recoverable job states", () => {
    expect(recordingStatusForStartFailure("MODEL_MISSING")).toBe("blocked_setup_required");
    expect(recordingStatusForStartFailure("FALLBACK_DISABLED")).toBe("blocked_setup_required");
    expect(recordingStatusForStartFailure("SERVER_UNAVAILABLE")).toBe("blocked_server_unavailable");
    expect(recordingStatusForStartFailure("SIGN_IN_REQUIRED")).toBe("blocked_sign_in_required");
    expect(recordingStatusForStartFailure("BUSY")).toBe("failed");
    expect(recordingStatusForStartFailure()).toBe("failed");
  });

  it("guards workspace event payloads at runtime", () => {
    expect(isWorkspaceView("home")).toBe(true);
    expect(isWorkspaceView("polish")).toBe(true);
    expect(isWorkspaceView("details")).toBe(false);
    expect(isWorkspaceView(undefined)).toBe(false);
  });
});
