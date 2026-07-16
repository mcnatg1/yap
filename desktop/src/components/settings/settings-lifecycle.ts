import type { LiveSessionStatus } from "@/lib/live-session";
import type { FallbackModelStatus, FallbackModelView } from "@/lib/setup-model";

export type FallbackLifecycleActionId =
  | "install"
  | "cancel"
  | "open-folder"
  | "reinstall"
  | "verify"
  | "disable"
  | "remove"
  | "repair"
  | "enable"
  | "retry";

export type FallbackLifecycleAction = {
  disabled: boolean;
  id: FallbackLifecycleActionId;
  label: string;
};

export type FallbackLifecycleProjection = {
  detail?: string;
  primaryAction?: FallbackLifecycleAction;
  secondaryActions: FallbackLifecycleAction[];
  value: string;
};

const fallbackCommandLabels: Record<FallbackLifecycleActionId, string> = {
  install: "Install",
  cancel: "Cancel",
  "open-folder": "Open folder",
  reinstall: "Reinstall",
  verify: "Verify",
  disable: "Disable",
  remove: "Remove",
  repair: "Repair",
  enable: "Enable",
  retry: "Retry",
};

const liveLockedStatuses = new Set<LiveSessionStatus>([
  "armed",
  "listening",
  "speaking",
  "settling",
  "saving",
]);

export function liveSettingsLocked(status: LiveSessionStatus) {
  return liveLockedStatuses.has(status);
}

export function projectLiveOverlayAction(status: LiveSessionStatus, liveBusy: boolean) {
  if (status === "saving") {
    return {
      disabled: true,
      label: "Saving",
    };
  }
  return {
    disabled: liveBusy,
    label: liveSettingsLocked(status) ? "Stop" : "Start",
  };
}

function fallbackDownloadLabel(view: FallbackModelView) {
  const percent = typeof view.progressPercent === "number"
    ? Math.max(0, Math.min(100, Math.round(view.progressPercent)))
    : null;
  return percent === null ? "Downloading" : `Downloading ${percent}%`;
}

function fallbackDownloadDetail(view: FallbackModelView) {
  if (typeof view.speedMbps === "number" && Number.isFinite(view.speedMbps) && view.speedMbps > 0) {
    return `${view.speedMbps.toFixed(view.speedMbps >= 10 ? 0 : 1)} Mbps`;
  }
  return view.message ?? undefined;
}

function createAction(
  id: FallbackLifecycleActionId,
  disabled: boolean,
) {
  return {
    disabled,
    id,
    label: fallbackCommandLabels[id],
  } satisfies FallbackLifecycleAction;
}

export function projectFallbackLifecycle(
  fallbackModel: FallbackModelView | null,
  options: {
    commandPending: boolean;
    liveStatus: LiveSessionStatus;
  },
): FallbackLifecycleProjection {
  if (!fallbackModel) {
    return {
      secondaryActions: [],
      value: "Checking",
    };
  }

  const liveActive = liveSettingsLocked(options.liveStatus);
  const toggleDisabled = options.commandPending || liveActive;
  const removeDisabled = options.commandPending || liveActive;
  const verifyDisabled = options.commandPending || liveActive;
  const installDisabled = options.commandPending || liveActive;
  const openFolder = createAction("open-folder", false);

  const secondaryActions = (status: FallbackModelStatus) => {
    switch (status) {
      case "missing":
      case "downloading":
      case "verifying":
        return [openFolder];
      case "ready":
        return [
          createAction("verify", verifyDisabled),
          createAction("disable", toggleDisabled),
          createAction("remove", removeDisabled),
          openFolder,
        ];
      case "corrupted":
      case "disabled":
      case "error":
        return [createAction("remove", removeDisabled), openFolder];
    }
  };

  switch (fallbackModel.status) {
    case "missing":
      return {
        detail: "Local fallback is not installed.",
        primaryAction: createAction("install", installDisabled),
        secondaryActions: secondaryActions("missing"),
        value: "Not installed",
      };
    case "downloading":
      return {
        detail: fallbackDownloadDetail(fallbackModel),
        primaryAction: createAction("cancel", fallbackModel.status !== "downloading"),
        secondaryActions: secondaryActions("downloading"),
        value: fallbackDownloadLabel(fallbackModel),
      };
    case "verifying":
      return {
        detail: "Verifying files.",
        secondaryActions: secondaryActions("verifying"),
        value: "Verifying files",
      };
    case "ready":
      return {
        detail: "Ready.",
        primaryAction: createAction("reinstall", installDisabled),
        secondaryActions: secondaryActions("ready"),
        value: "Ready",
      };
    case "corrupted":
      return {
        detail: "Files failed verification.",
        primaryAction: createAction("repair", installDisabled),
        secondaryActions: secondaryActions("corrupted"),
        value: "Files failed verification.",
      };
    case "disabled":
      return {
        detail: "Disabled.",
        primaryAction: createAction("enable", toggleDisabled),
        secondaryActions: secondaryActions("disabled"),
        value: "Disabled",
      };
    case "error":
      return {
        detail: fallbackModel.message ?? "Local fallback needs attention.",
        primaryAction: createAction("retry", installDisabled),
        secondaryActions: secondaryActions("error"),
        value: "Needs attention",
      };
  }
}
