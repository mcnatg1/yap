import { describe, expect, it } from "vitest";

import { liveSettingsLocked, projectFallbackLifecycle, projectLiveOverlayAction } from "@/components/panels/app-sheets";
import {
  createInitialPipelineState,
  deriveSetupState,
  deriveSetupStateFromFallbackModel,
  isFallbackModelBusy,
  isRecordingActive,
  isRecordingFinished,
  isWorkspaceView,
  fallbackModelLabel,
  serverConnectionLabel,
  setupStateLabel,
} from "@/lib/app-types";
import { serverCanRouteImportedRecording, serverCanRouteLive } from "@/server";

describe("client recording workflow projection", () => {
  const baseFallbackModel = {
    id: "nemotron-3.5-asr-streaming-0.6b-1120ms-int8" as const,
    label: "Nemotron 3.5 ASR Streaming 0.6B INT8",
    modelsDir: "C:\\models",
    status: "missing" as const,
  };

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
    expect(deriveSetupStateFromFallbackModel("downloading", true)).toBe("fallback_installing");
    expect(deriveSetupStateFromFallbackModel("verifying", true)).toBe("fallback_installing");
    expect(deriveSetupStateFromFallbackModel("corrupted", true)).toBe("fallback_missing");
    expect(deriveSetupStateFromFallbackModel("missing", false)).toBe("fallback_disabled");
    expect(deriveSetupStateFromFallbackModel("error", true)).toBe("setup_error");

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

  it("fails closed when projecting server capabilities", () => {
    const readyWithoutCapabilities = {
      state: "ready" as const,
      checkedAtMs: 10,
      retryAtMs: null,
      apiVersion: "1",
      capabilities: { batchJobs: false, liveStreaming: false, jobStatus: false },
      errorCode: null,
    };

    expect(serverCanRouteImportedRecording(readyWithoutCapabilities)).toBe(false);
    expect(serverCanRouteLive(readyWithoutCapabilities)).toBe(false);
    expect(serverCanRouteImportedRecording({
      ...readyWithoutCapabilities,
      capabilities: { ...readyWithoutCapabilities.capabilities, batchJobs: true },
    })).toBe(true);
    expect(serverCanRouteLive({
      ...readyWithoutCapabilities,
      capabilities: { ...readyWithoutCapabilities.capabilities, liveStreaming: true },
    })).toBe(true);
    expect(serverCanRouteLive({
      ...readyWithoutCapabilities,
      state: "offline",
      capabilities: { ...readyWithoutCapabilities.capabilities, liveStreaming: true },
    })).toBe(false);
  });

  it("labels the pinned local fallback model clearly", () => {
    expect(fallbackModelLabel("Nemotron 3.5 ASR Streaming 0.6B INT8")).toBe("Nemotron 3.5 ASR Streaming 0.6B INT8");
    expect(fallbackModelLabel("custom.gguf")).toBe("custom");
  });

  it("keeps queued, active server, and finished statuses distinct", () => {
    expect(isRecordingActive("server_processing")).toBe(true);
    expect(isRecordingActive("queued_server")).toBe(false);
    expect(isRecordingFinished("complete")).toBe(true);
    expect(isRecordingFinished("partial")).toBe(true);
    expect(isRecordingFinished("queued_server")).toBe(false);
  });

  it("guards workspace event payloads at runtime", () => {
    expect(isWorkspaceView("home")).toBe(true);
    expect(isWorkspaceView("polish")).toBe(true);
    expect(isWorkspaceView("details")).toBe(false);
    expect(isWorkspaceView(undefined)).toBe(false);
  });

  it("treats downloading and verifying fallback states as busy", () => {
    expect(isFallbackModelBusy({ status: "downloading" }, false)).toBe(true);
    expect(isFallbackModelBusy({ status: "verifying" }, false)).toBe(true);
    expect(isFallbackModelBusy({ status: "ready" }, true)).toBe(true);
    expect(isFallbackModelBusy({ status: "ready" }, false)).toBe(false);
    expect(isFallbackModelBusy(undefined, false)).toBe(false);
  });

  it("projects the full fallback lifecycle action matrix", () => {
    expect(projectFallbackLifecycle({ ...baseFallbackModel, status: "missing" }, { commandPending: false, liveStatus: "idle" })).toMatchObject({
      detail: "Local fallback is not installed.",
      value: "Not installed",
      primaryAction: { id: "install", disabled: false, label: "Install" },
      secondaryActions: [{ id: "open-folder", disabled: false, label: "Open folder" }],
    });

    expect(projectFallbackLifecycle({
      ...baseFallbackModel,
      progressPercent: 42,
      speedMbps: 8.4,
      status: "downloading",
    }, { commandPending: false, liveStatus: "idle" })).toMatchObject({
      detail: "8.4 Mbps",
      value: "Downloading 42%",
      primaryAction: { id: "cancel", disabled: false, label: "Cancel" },
      secondaryActions: [{ id: "open-folder", disabled: false, label: "Open folder" }],
    });

    const verifying = projectFallbackLifecycle(
      { ...baseFallbackModel, status: "verifying" },
      { commandPending: false, liveStatus: "idle" },
    );
    expect(verifying).toMatchObject({
      detail: "Verifying files.",
      value: "Verifying files",
      secondaryActions: [{ id: "open-folder", disabled: false, label: "Open folder" }],
    });
    expect(verifying.primaryAction).toBeUndefined();

    expect(projectFallbackLifecycle({ ...baseFallbackModel, status: "ready" }, { commandPending: false, liveStatus: "idle" })).toMatchObject({
      detail: "Ready.",
      value: "Ready",
      primaryAction: { id: "reinstall", disabled: false, label: "Reinstall" },
      secondaryActions: [
        { id: "verify", disabled: false, label: "Verify" },
        { id: "disable", disabled: false, label: "Disable" },
        { id: "remove", disabled: false, label: "Remove" },
        { id: "open-folder", disabled: false, label: "Open folder" },
      ],
    });

    expect(projectFallbackLifecycle({ ...baseFallbackModel, status: "corrupted" }, { commandPending: false, liveStatus: "idle" })).toMatchObject({
      detail: "Files failed verification.",
      value: "Files failed verification.",
      primaryAction: { id: "repair", disabled: false, label: "Repair" },
      secondaryActions: [
        { id: "remove", disabled: false, label: "Remove" },
        { id: "open-folder", disabled: false, label: "Open folder" },
      ],
    });

    expect(projectFallbackLifecycle({ ...baseFallbackModel, status: "disabled" }, { commandPending: false, liveStatus: "idle" })).toMatchObject({
      detail: "Disabled.",
      value: "Disabled",
      primaryAction: { id: "enable", disabled: false, label: "Enable" },
      secondaryActions: [
        { id: "remove", disabled: false, label: "Remove" },
        { id: "open-folder", disabled: false, label: "Open folder" },
      ],
    });

    expect(projectFallbackLifecycle({
      ...baseFallbackModel,
      message: "Checksum mismatch",
      status: "error",
    }, { commandPending: false, liveStatus: "idle" })).toMatchObject({
      detail: "Checksum mismatch",
      value: "Needs attention",
      primaryAction: { id: "retry", disabled: false, label: "Retry" },
      secondaryActions: [
        { id: "remove", disabled: false, label: "Remove" },
        { id: "open-folder", disabled: false, label: "Open folder" },
      ],
    });

    expect(projectFallbackLifecycle({
      ...baseFallbackModel,
      message: undefined,
      status: "error",
    }, { commandPending: false, liveStatus: "idle" })).toMatchObject({
      detail: "Local fallback needs attention.",
    });
  });

  it("treats saving as an active settings lock", () => {
    expect(liveSettingsLocked("saving")).toBe(true);
    expect(liveSettingsLocked("idle")).toBe(false);
  });

  it("keeps the settings Stop action available while live is active", () => {
    expect(projectLiveOverlayAction("speaking", false)).toEqual({
      disabled: false,
      label: "Stop",
    });
    expect(projectLiveOverlayAction("speaking", true)).toEqual({
      disabled: true,
      label: "Stop",
    });
    expect(projectLiveOverlayAction("idle", false)).toEqual({
      disabled: false,
      label: "Start",
    });
    expect(projectLiveOverlayAction("saving", false)).toEqual({
      disabled: true,
      label: "Saving",
    });
  });

  it("locks install, remove, and verify while live is active but keeps cancel during downloads", () => {
    const ready = projectFallbackLifecycle(
      { ...baseFallbackModel, status: "ready" },
      { commandPending: false, liveStatus: "saving" },
    );

    expect(ready.primaryAction).toMatchObject({ id: "reinstall", disabled: true });
    expect(ready.secondaryActions.find((action) => action.id === "verify")).toMatchObject({ disabled: true });
    expect(ready.secondaryActions.find((action) => action.id === "remove")).toMatchObject({ disabled: true });
    expect(ready.secondaryActions.find((action) => action.id === "open-folder")).toMatchObject({ disabled: false });

    const downloading = projectFallbackLifecycle(
      { ...baseFallbackModel, status: "downloading" },
      { commandPending: false, liveStatus: "saving" },
    );
    const cancelling = projectFallbackLifecycle(
      { ...baseFallbackModel, status: "downloading" },
      { commandPending: true, liveStatus: "idle" },
    );

    expect(downloading.primaryAction).toMatchObject({ id: "cancel", disabled: false });
    expect(cancelling.primaryAction).toMatchObject({ id: "cancel", disabled: false });
  });
});
