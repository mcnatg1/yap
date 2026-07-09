import type { FallbackModelView, RecordingJobView, SetupState } from "@/lib/app-types";

export function fallbackStatusText(view: FallbackModelView, enabled: boolean) {
  switch (view.status) {
    case "downloading":
      return view.message ?? "Installing local fallback";
    case "verifying":
      return view.message ?? "Verifying local fallback";
    case "ready":
      return "Transcription engine ready";
    case "disabled":
      return "Local fallback disabled";
    case "error":
      return view.message ?? "Local fallback needs attention";
    case "missing":
    case "corrupted":
      return enabled ? "Local fallback model missing" : "Local fallback disabled";
  }
}

export function shouldOpenSetupPrompt({
  alreadyPrompted,
  fallbackEnabled,
  setupState,
  skipped,
}: {
  alreadyPrompted: boolean;
  fallbackEnabled: boolean;
  setupState: SetupState;
  skipped: boolean;
}) {
  return fallbackEnabled && setupState !== "fallback_ready" && !alreadyPrompted && !skipped;
}

export function unblockFallbackReadyQueue(items: RecordingJobView[]) {
  return items.map((item) =>
    item.status === "blocked_setup_required"
      ? {
          ...item,
          error: undefined,
          pipeline: {
            ...item.pipeline,
            transcription: "notStarted" as const,
          },
          status: "queued_local_fallback" as const,
        }
      : item,
  );
}
