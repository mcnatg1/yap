import {
  deriveSetupStateFromFallbackModel,
  type FallbackModelView,
  type SetupState,
} from "@/lib/setup-model";

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

export type FallbackModelStateOverrides = {
  authText?: string;
  engineReady?: boolean;
  fallbackEnabled?: boolean;
  modelInstalled?: boolean;
  statusText?: string;
};

export function projectFallbackModelState({
  alreadyPrompted,
  currentFallbackEnabled,
  currentModelInstalled,
  overrides = {},
  skipped,
  view,
}: {
  alreadyPrompted: boolean;
  currentFallbackEnabled: boolean;
  currentModelInstalled: boolean;
  overrides?: FallbackModelStateOverrides;
  skipped: boolean;
  view: FallbackModelView;
}) {
  const fallbackEnabled = overrides.fallbackEnabled
    ?? (view.status === "ready" ? true : view.status === "disabled" ? false : currentFallbackEnabled);
  const modelInstalled = overrides.modelInstalled
    ?? (
      view.status === "ready" || view.status === "disabled" || view.status === "corrupted"
        ? true
        : view.status === "missing"
          ? false
          : currentModelInstalled
    );
  const engineReady = overrides.engineReady ?? view.status === "ready";
  const setupState = deriveSetupStateFromFallbackModel(view.status, fallbackEnabled);
  const requestSetupPrompt = shouldOpenSetupPrompt({
    alreadyPrompted,
    fallbackEnabled,
    setupState,
    skipped,
  });

  return {
    auth: overrides.authText ?? (engineReady ? "Ready" : "Setup"),
    engineReady,
    fallbackEnabled,
    modelInstalled,
    requestSetupPrompt,
    setupPrompted: alreadyPrompted || requestSetupPrompt,
    setupState,
    status: overrides.statusText ?? fallbackStatusText(view, fallbackEnabled),
  };
}
