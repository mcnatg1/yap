import { describe, expect, it } from "vitest";

import {
  createInitialPipelineState,
  deriveSetupStateFromFallbackModel,
  type FallbackModelStatus,
  type FallbackModelView,
  type RecordingJobView,
} from "@/lib/app-types";
import {
  fallbackStatusText,
  projectFallbackModelState,
  shouldOpenSetupPrompt,
  unblockFallbackReadyQueue,
} from "@/lib/setup-model-state";

function fallbackView(status: FallbackModelStatus, message?: string): FallbackModelView {
  return {
    id: "nemotron-3.5-asr-streaming-0.6b-1120ms-int8",
    label: "Nemotron",
    message,
    modelsDir: "C:/Yap/models",
    status,
  };
}

function job(status: RecordingJobView["status"], id = 1): RecordingJobView {
  return {
    id,
    intent: "recording",
    name: `take-${id}.wav`,
    path: `C:/take-${id}.wav`,
    pipeline: { ...createInitialPipelineState(), transcription: "error" },
    route: "localFallback",
    status,
  };
}

describe("setup model state", () => {
  it("uses model progress messages before generic status text", () => {
    expect(fallbackStatusText(fallbackView("downloading", "Fetching model"), true)).toBe("Fetching model");
    expect(fallbackStatusText(fallbackView("verifying", "Checking files"), true)).toBe("Checking files");
  });

  it("labels ready, disabled, and missing fallback states", () => {
    expect(fallbackStatusText(fallbackView("ready"), true)).toBe("Transcription engine ready");
    expect(fallbackStatusText(fallbackView("disabled"), true)).toBe("Local fallback disabled");
    expect(fallbackStatusText(fallbackView("missing"), true)).toBe("Local fallback model missing");
    expect(fallbackStatusText(fallbackView("corrupted"), false)).toBe("Local fallback disabled");
  });

  it("does not prompt when fallback is disabled", () => {
    expect(
      shouldOpenSetupPrompt({
        alreadyPrompted: false,
        fallbackEnabled: false,
        setupState: "fallback_missing",
        skipped: false,
      }),
    ).toBe(false);
  });

  it("does not prompt when fallback is ready", () => {
    expect(
      shouldOpenSetupPrompt({
        alreadyPrompted: false,
        fallbackEnabled: true,
        setupState: deriveSetupStateFromFallbackModel("ready", true),
        skipped: false,
      }),
    ).toBe(false);
  });

  it("prompts once when fallback is enabled and missing", () => {
    expect(
      shouldOpenSetupPrompt({
        alreadyPrompted: false,
        fallbackEnabled: true,
        setupState: "fallback_missing",
        skipped: false,
      }),
    ).toBe(true);
    expect(
      shouldOpenSetupPrompt({
        alreadyPrompted: true,
        fallbackEnabled: true,
        setupState: "fallback_missing",
        skipped: false,
      }),
    ).toBe(false);
  });

  it("unblocks only fallback setup-blocked jobs when fallback becomes ready", () => {
    const queuedServer = job("queued_server", 1);
    const blocked = job("blocked_setup_required", 2);

    const next = unblockFallbackReadyQueue([queuedServer, blocked]);

    expect(next[0]).toBe(queuedServer);
    expect(next[1]).toMatchObject({
      error: undefined,
      status: "queued_local_fallback",
      pipeline: { transcription: "notStarted" },
    });
  });

  it.each([
    {
      currentEnabled: false,
      currentInstalled: false,
      expected: {
        auth: "Ready",
        engineReady: true,
        fallbackEnabled: true,
        modelInstalled: true,
        requestQueueUnblock: true,
        setupState: "fallback_ready",
        status: "Transcription engine ready",
      },
      modelStatus: "ready" as const,
    },
    {
      currentEnabled: true,
      currentInstalled: false,
      expected: {
        auth: "Setup",
        engineReady: false,
        fallbackEnabled: false,
        modelInstalled: true,
        requestQueueUnblock: false,
        setupState: "fallback_disabled",
        status: "Local fallback disabled",
      },
      modelStatus: "disabled" as const,
    },
    {
      currentEnabled: true,
      currentInstalled: true,
      expected: {
        auth: "Setup",
        engineReady: false,
        fallbackEnabled: true,
        modelInstalled: false,
        requestQueueUnblock: false,
        setupState: "fallback_missing",
        status: "Local fallback model missing",
      },
      modelStatus: "missing" as const,
    },
    {
      currentEnabled: false,
      currentInstalled: false,
      expected: {
        auth: "Setup",
        engineReady: false,
        fallbackEnabled: false,
        modelInstalled: true,
        requestQueueUnblock: false,
        setupState: "fallback_disabled",
        status: "Local fallback disabled",
      },
      modelStatus: "corrupted" as const,
    },
    {
      currentEnabled: false,
      currentInstalled: true,
      expected: {
        auth: "Setup",
        engineReady: false,
        fallbackEnabled: false,
        modelInstalled: true,
        requestQueueUnblock: false,
        setupState: "fallback_disabled",
        status: "Fetching model",
      },
      modelStatus: "downloading" as const,
    },
  ])(
    "projects $modelStatus with current-value precedence",
    ({ currentEnabled, currentInstalled, expected, modelStatus }) => {
      expect(
        projectFallbackModelState({
          alreadyPrompted: true,
          currentFallbackEnabled: currentEnabled,
          currentModelInstalled: currentInstalled,
          skipped: false,
          view: fallbackView(modelStatus, modelStatus === "downloading" ? "Fetching model" : undefined),
        }),
      ).toMatchObject(expected);
    },
  );

  it("applies explicit setup overrides before model-derived values", () => {
    expect(
      projectFallbackModelState({
        alreadyPrompted: true,
        currentFallbackEnabled: false,
        currentModelInstalled: false,
        overrides: {
          authText: "Checking native setup",
          engineReady: false,
          fallbackEnabled: false,
          modelInstalled: false,
          statusText: "Native setup pending",
        },
        skipped: false,
        view: fallbackView("ready"),
      }),
    ).toMatchObject({
      auth: "Checking native setup",
      engineReady: false,
      fallbackEnabled: false,
      modelInstalled: false,
      requestQueueUnblock: false,
      setupState: "fallback_disabled",
      status: "Native setup pending",
    });
  });

  it("requests setup once and carries the prompted state forward", () => {
    const first = projectFallbackModelState({
      alreadyPrompted: false,
      currentFallbackEnabled: true,
      currentModelInstalled: false,
      skipped: false,
      view: fallbackView("missing"),
    });
    const second = projectFallbackModelState({
      alreadyPrompted: first.setupPrompted,
      currentFallbackEnabled: first.fallbackEnabled,
      currentModelInstalled: first.modelInstalled,
      skipped: false,
      view: fallbackView("missing"),
    });

    expect(first.requestSetupPrompt).toBe(true);
    expect(first.setupPrompted).toBe(true);
    expect(second.requestSetupPrompt).toBe(false);
  });

  it("keeps setup prompting suppressed after Skip", () => {
    expect(
      projectFallbackModelState({
        alreadyPrompted: false,
        currentFallbackEnabled: true,
        currentModelInstalled: false,
        skipped: true,
        view: fallbackView("missing"),
      }),
    ).toMatchObject({ requestSetupPrompt: false, setupPrompted: false });
  });
});
