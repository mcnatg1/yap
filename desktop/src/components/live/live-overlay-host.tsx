import { useEffect, useState } from "react";

import { LiveOverlay } from "@/components/live/live-overlay";
import {
  listenLiveSession,
  liveStatus,
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
      onStart={() => void startLiveSession().then(setView)}
      onStop={() => void stopLiveSession().then(setView)}
      view={view}
    />
  );
}
