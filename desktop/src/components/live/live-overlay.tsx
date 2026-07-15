import gsap from "gsap";
import { useLayoutEffect, useRef } from "react";

import { LiveOverlayContent } from "@/components/live/live-overlay-views";
import { useLiveOverlayPresentation } from "@/components/live/use-live-overlay-presentation";
import { usePrefersReducedMotion } from "@/components/live/use-prefers-reduced-motion";
import type { LiveOverlayView } from "@/lib/live-session";
import { cn } from "@/lib/utils";

type LiveOverlayProps = {
  onOpenScratch?: () => void;
  onOpenTransform?: () => void;
  onRetry?: () => void;
  onStart?: () => void;
  onStop?: () => void;
  view: LiveOverlayView;
};

export function LiveOverlay({
  onOpenScratch,
  onOpenTransform,
  onRetry,
  onStart,
  onStop,
  view,
}: LiveOverlayProps) {
  const prefersReducedMotion = usePrefersReducedMotion();
  const contentRef = useRef<HTMLDivElement>(null);
  const {
    hiddenIdle,
    model,
    native,
    openIdleIsland,
    rootFrameStyle,
    scheduleIdleCollapse,
    surface,
  } = useLiveOverlayPresentation(view);

  useOverlayTransition(contentRef, surface, prefersReducedMotion);

  if (hiddenIdle) return null;

  return (
    <div
      className={cn(
        "live-overlay-root h-full w-full overflow-hidden bg-transparent p-0",
        model.phase === "idle" ? "pointer-events-auto" : "pointer-events-none",
      )}
      data-overlay-phase={model.phase}
      data-overlay-surface={surface}
      data-testid="live-overlay-root"
      onPointerEnter={() => {
        if (model.phase === "idle") openIdleIsland();
      }}
      onMouseLeave={() => {
        if (surface === "expanded") scheduleIdleCollapse();
      }}
      onPointerOut={(event) => {
        if (event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget)) return;
        if (surface === "expanded") scheduleIdleCollapse();
      }}
      style={rootFrameStyle}
    >
      <div
        className="pointer-events-auto h-full w-full text-white"
        data-testid="live-overlay-island"
        style={{
          backgroundColor: "black",
          borderRadius: native ? undefined : 14,
          overflow: "hidden",
        }}
      >
        <div className="h-full w-full" ref={contentRef}>
          <LiveOverlayContent
            model={model}
            onOpenScratch={onOpenScratch}
            onOpenTransform={onOpenTransform}
            onRetry={onRetry}
            onStart={onStart}
            onStop={onStop}
            prefersReducedMotion={prefersReducedMotion}
            surface={surface}
          />
        </div>
      </div>
    </div>
  );
}

function useOverlayTransition(
  contentRef: React.RefObject<HTMLDivElement | null>,
  surface: string,
  prefersReducedMotion: boolean,
) {
  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    gsap.killTweensOf(content);

    if (prefersReducedMotion) {
      gsap.set(content, { opacity: 1, y: 0 });
      return;
    }
    gsap.fromTo(
      content,
      { opacity: 0.72, y: -2 },
      { duration: 0.12, ease: "power2.out", opacity: 1, overwrite: true, y: 0 },
    );
    return () => gsap.killTweensOf(content);
  }, [contentRef, prefersReducedMotion, surface]);
}
