import { useEffect, useState, type Dispatch, type SetStateAction } from "react";

import { LiveOverlay } from "@/components/live/live-overlay";
import { emitLiveOverlayLevel } from "@/components/live/live-waveform";
import {
  listenLiveLevel,
  listenLiveOverlaySession,
  liveOverlayStatus,
  showMainWorkspace,
  startLiveOverlaySession,
  stopLiveOverlaySession,
} from "@/live";
import type { LiveOverlayView } from "@/lib/app-types";

const liveStatuses = ["idle", "armed", "listening", "speaking", "settling", "blocked", "saving"] as const;
const liveCaptureModes = ["pushToTalk", "toggle"] as const;
const liveVisibilities = ["enabled", "hidden"] as const;
const previewEventName = "yap-live-overlay-preview";

const previewLiveView: LiveOverlayView = {
  captureMode: "pushToTalk",
  hasFinalText: false,
  status: "idle",
  visibility: "enabled",
};

export function LiveOverlayHost() {
  const previewMode = isPreviewMode();
  const [view, setView] = useState<LiveOverlayView>(() => previewMode ? previewLiveViewFromSearch() : previewLiveView);

  useEffect(() => {
    if (previewMode) {
      const handlePreviewEvent = (event: Event) => {
        const detail = (event as CustomEvent<Partial<LiveOverlayView>>).detail ?? {};
        setView((current) => ({ ...current, ...detail }));
      };

      window.addEventListener(previewEventName, handlePreviewEvent);
      return () => window.removeEventListener(previewEventName, handlePreviewEvent);
    }

    let cancelled = false;
    let refreshEpoch = 0;
    let unlistenLevel: (() => void) | undefined;
    let unlisten: (() => void) | undefined;
    const refreshFromNative = () => {
      const epoch = ++refreshEpoch;
      void liveOverlayStatus().then((nativeView) => {
        if (!cancelled && epoch === refreshEpoch) setView(nativeView);
      }).catch(() => undefined);
    };
    // Install the invalidation listener before the first read so a native
    // transition cannot land between an initial snapshot and subscription.
    void listenLiveOverlaySession(refreshFromNative).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlisten = stop;
      refreshFromNative();
    }).catch(() => {
      if (!cancelled) refreshFromNative();
    });
    void listenLiveLevel((level) => {
      emitLiveOverlayLevel(level.level ?? 0);
    }).then((stop) => {
      if (cancelled) {
        stop();
        return;
      }
      unlistenLevel = stop;
    });

    return () => {
      cancelled = true;
      unlistenLevel?.();
      unlisten?.();
    };
  }, [previewMode]);

  return (
    <LiveOverlay
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
        void stopLiveOverlaySession().then(setView);
      }}
      view={view}
    />
  );
}

function startToggleSession(previewMode: boolean, setView: Dispatch<SetStateAction<LiveOverlayView>>) {
  if (previewMode) {
    setView((current) => ({
      ...current,
      activeCaptureMode: "toggle",
      level: 0.56,
      status: "listening",
    }));
    return;
  }
  void startLiveOverlaySession("toggle").then(setView);
}

function isPreviewMode() {
  return import.meta.env.DEV && new URLSearchParams(window.location.search).get("preview") === "live-overlay";
}

function previewLiveViewFromSearch(): LiveOverlayView {
  const params = new URLSearchParams(window.location.search);
  return {
    captureMode: oneOf(params.get("captureMode"), liveCaptureModes) ?? previewLiveView.captureMode,
    activeCaptureMode: oneOf(params.get("activeCaptureMode"), liveCaptureModes),
    error: params.get("error") ?? undefined,
    hasFinalText: params.get("hasFinalText") === "true",
    level: numberParam(params.get("level")),
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
