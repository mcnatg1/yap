import { useEffect, useState } from "react";

import { LiveOverlay } from "@/components/live/live-overlay";
import {
  listenLiveSession,
  liveStatus,
  setLiveCaptureMode,
  showMainWorkspace,
  startLiveSession,
  stopLiveSession,
} from "@/live";
import type { LiveSessionView } from "@/lib/app-types";

const previewLiveView: LiveSessionView = {
  captureMode: "pushToTalk",
  hotkey: "Ctrl+Shift+Space",
  route: "none",
  status: "idle",
  visibility: "enabled",
};

export function LiveOverlayHost() {
  const [view, setView] = useState<LiveSessionView>(previewLiveView);

  useEffect(() => {
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
  }, []);

  return (
    <LiveOverlay
      onCopyLast={() => {
        const text = view.finalText?.trim();
        if (!text) return;
        void navigator.clipboard.writeText(text).catch(() => undefined);
      }}
      onOpenScratch={() => void showMainWorkspace("home")}
      onOpenTransform={() => void showMainWorkspace("polish")}
      onRetry={() => void startLiveSession().then(setView)}
      onStart={() => {
        void setLiveCaptureMode("toggle")
          .catch(() => view)
          .then(() => startLiveSession())
          .then(setView);
      }}
      onStop={() => void stopLiveSession().then(setView)}
      view={view}
    />
  );
}
