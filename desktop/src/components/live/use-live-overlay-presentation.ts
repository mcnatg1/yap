import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";

import {
  collapseGraceMs,
  modelFromLiveView,
  overlaySurface,
  previewOverlayFrame,
  successVisibleMs,
} from "@/components/live/live-overlay-state";
import { createNativeSurfaceSync } from "@/components/live/native-surface-sync";
import type { LiveOverlayView } from "@/lib/live-session";

const setNativeOverlaySurface = createNativeSurfaceSync(async ({ surface }) => {
  if (!isTauri()) return;
  await invoke("set_live_overlay_surface", { surface });
});

export function useLiveOverlayPresentation(view: LiveOverlayView) {
  const model = modelFromLiveView(view);
  const [expanded, setExpanded] = useState(false);
  const [successVisible, setSuccessVisible] = useState(false);
  const previousStatusRef = useRef(view.status);
  const collapseTimerRef = useRef<number | undefined>(undefined);
  const successTimerRef = useRef<number | undefined>(undefined);
  const native = isTauri();
  const hasCopyableFinal = model.hasFinalText;
  const surface = overlaySurface(model, expanded, successVisible && hasCopyableFinal);
  const hiddenIdle = view.visibility === "hidden" && model.phase === "idle";
  const rootFrameStyle: CSSProperties | undefined = native ? undefined : previewOverlayFrame(surface);

  const clearSuccessTimer = useCallback(() => {
    if (successTimerRef.current === undefined) return;
    window.clearTimeout(successTimerRef.current);
    successTimerRef.current = undefined;
  }, []);

  const cancelCollapse = useCallback(() => {
    if (collapseTimerRef.current === undefined) return;
    window.clearTimeout(collapseTimerRef.current);
    collapseTimerRef.current = undefined;
  }, []);

  const openIdleIsland = useCallback(() => {
    cancelCollapse();
    setExpanded(true);
  }, [cancelCollapse]);

  const scheduleIdleCollapse = useCallback(() => {
    cancelCollapse();
    collapseTimerRef.current = window.setTimeout(() => {
      collapseTimerRef.current = undefined;
      setExpanded(false);
    }, collapseGraceMs);
  }, [cancelCollapse]);

  useEffect(() => {
    if (model.phase === "idle") return;
    cancelCollapse();
    setExpanded(false);
  }, [cancelCollapse, model.phase]);

  useLayoutEffect(() => {
    if (!hiddenIdle) return;
    cancelCollapse();
    setExpanded(false);
  }, [cancelCollapse, hiddenIdle]);

  useEffect(() => {
    const previousStatus = previousStatusRef.current;
    previousStatusRef.current = view.status;
    if (view.status !== "idle") {
      clearSuccessTimer();
      setSuccessVisible(false);
    } else if (previousStatus !== "idle" && hasCopyableFinal) {
      clearSuccessTimer();
      setSuccessVisible(true);
      successTimerRef.current = window.setTimeout(() => {
        successTimerRef.current = undefined;
        setSuccessVisible(false);
      }, successVisibleMs);
    }
  }, [clearSuccessTimer, hasCopyableFinal, view.status]);

  useEffect(() => {
    if (hiddenIdle) return;
    setNativeOverlaySurface({ surface });
  }, [hiddenIdle, surface]);

  useEffect(() => {
    return () => {
      cancelCollapse();
      clearSuccessTimer();
    };
  }, [cancelCollapse, clearSuccessTimer]);

  return {
    hiddenIdle,
    model,
    native,
    openIdleIsland,
    rootFrameStyle,
    scheduleIdleCollapse,
    surface,
  };
}
