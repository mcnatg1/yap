import { useEffect, useState, type Dispatch, type SetStateAction } from "react";

import { LiveOverlay } from "@/components/live/live-overlay";
import {
  listenLiveSession,
  liveStatus,
  showMainWorkspace,
  startLiveSession,
  stopLiveSession,
} from "@/live";
import type { LiveSessionView } from "@/lib/app-types";

const liveStatuses = ["idle", "armed", "listening", "speaking", "settling", "blocked", "saving"] as const;
const liveCaptureModes = ["pushToTalk", "toggle"] as const;
const liveRoutes = ["serverLive", "localFallback", "blocked", "none"] as const;
const liveVisibilities = ["enabled", "hidden"] as const;
const previewEventName = "yap-live-overlay-preview";

const previewLiveView: LiveSessionView = {
  captureMode: "pushToTalk",
  hotkey: "Ctrl+Shift+Space",
  route: "none",
  status: "idle",
  visibility: "enabled",
};

export function LiveOverlayHost() {
  const previewMode = isPreviewMode();
  const [view, setView] = useState<LiveSessionView>(() => previewMode ? previewLiveViewFromSearch() : previewLiveView);

  useEffect(() => {
    if (previewMode) {
      const handlePreviewEvent = (event: Event) => {
        const detail = (event as CustomEvent<Partial<LiveSessionView>>).detail ?? {};
        setView((current) => ({ ...current, ...detail }));
      };

      window.addEventListener(previewEventName, handlePreviewEvent);
      return () => window.removeEventListener(previewEventName, handlePreviewEvent);
    }

    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void liveStatus().then(setView).catch(() => undefined);
    void listenLiveSession(setView).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [previewMode]);

  return (
    <LiveOverlay
      onCopyLast={() => {
        const text = view.finalText?.trim();
        if (!text) return;
        void navigator.clipboard.writeText(text).catch(() => undefined);
      }}
      onOpenScratch={() => void showMainWorkspace("home")}
      onOpenTransform={() => void showMainWorkspace("polish")}
      onRetry={() => void startToggleSession(previewMode, setView)}
      onStart={() => {
        void startToggleSession(previewMode, setView);
      }}
      onStop={() => {
        if (previewMode) {
          setView((current) => ({ ...current, activeCaptureMode: undefined, level: 0, status: "saving" }));
          return;
        }
        void stopLiveSession().then(setView);
      }}
      view={view}
    />
  );
}

function startToggleSession(previewMode: boolean, setView: Dispatch<SetStateAction<LiveSessionView>>) {
  if (previewMode) {
    setView((current) => ({
      ...current,
      activeCaptureMode: "toggle",
      level: 0.56,
      route: "localFallback",
      status: "listening",
    }));
    return;
  }
  void startLiveSession("toggle").then(setView);
}

function isPreviewMode() {
  return import.meta.env.DEV && new URLSearchParams(window.location.search).get("preview") === "live-overlay";
}

function previewLiveViewFromSearch(): LiveSessionView {
  const params = new URLSearchParams(window.location.search);
  return {
    captureMode: oneOf(params.get("captureMode"), liveCaptureModes) ?? previewLiveView.captureMode,
    activeCaptureMode: oneOf(params.get("activeCaptureMode"), liveCaptureModes),
    error: params.get("error") ?? undefined,
    finalText: params.get("finalText") ?? undefined,
    hotkey: params.get("hotkey") ?? previewLiveView.hotkey,
    inputDeviceId: params.get("inputDeviceId") ?? undefined,
    inputDeviceLabel: params.get("inputDeviceLabel") ?? undefined,
    level: numberParam(params.get("level")),
    partialText: params.get("partialText") ?? undefined,
    route: oneOf(params.get("route"), liveRoutes) ?? previewLiveView.route,
    status: oneOf(params.get("status"), liveStatuses) ?? previewLiveView.status,
    visibility: oneOf(params.get("visibility"), liveVisibilities) ?? previewLiveView.visibility,
  };
}

function numberParam(value: string | null) {
  if (value === null) return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function oneOf<T extends string>(value: string | null, options: readonly T[]) {
  if (value === null) return undefined;
  return options.includes(value as T) ? (value as T) : undefined;
}
