import { Mic, MicOff, Save, Settings, Square } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { liveRouteLabel, liveStatusLabel, type LiveSessionView } from "@/lib/app-types";
import { cn } from "@/lib/utils";

type LiveOverlayProps = {
  onOpenSettings?: () => void;
  onSave?: () => void;
  onStart?: () => void;
  onStop?: () => void;
  view: LiveSessionView;
};

const startedStatuses = new Set<LiveSessionView["status"]>(["armed", "listening", "speaking", "settling"]);
const micHotStatuses = new Set<LiveSessionView["status"]>(["listening", "speaking", "settling"]);

export function LiveOverlay({ onOpenSettings, onSave, onStart, onStop, view }: LiveOverlayProps) {
  const [expanded, setExpanded] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const started = startedStatuses.has(view.status);
  const micHot = micHotStatuses.has(view.status);
  const blocked = view.status === "blocked";

  useEffect(() => {
    const node = rootRef.current;
    if (!node || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let cancelled = false;
    void import("gsap").then(({ gsap }) => {
      if (cancelled) return;
      gsap.fromTo(
        node,
        { opacity: 0.82, scale: expanded ? 0.98 : 1.02, y: expanded ? -4 : 0 },
        { opacity: 1, scale: 1, y: 0, duration: 0.16, ease: "power2.out" },
      );
    });

    return () => {
      cancelled = true;
    };
  }, [expanded, view.status]);

  if (view.visibility === "hidden") return null;

  return (
    <div className="flex h-screen items-start justify-center bg-transparent pt-2">
      <div
        ref={rootRef}
        className={cn(
          "live-glass max-w-[calc(100vw-16px)] border px-2 py-1.5 text-sm shadow-lg",
          expanded ? "w-[min(420px,calc(100vw-16px))] rounded-xl" : "rounded-full",
          started ? "border-primary/30" : "border-white/40",
          blocked ? "border-destructive/35" : "",
        )}
      >
        <button
          aria-expanded={expanded}
          aria-label={`${liveStatusLabel(view.status)}. ${expanded ? "Collapse" : "Expand"} live controls`}
          className="flex min-h-10 min-w-10 items-center gap-2 rounded-full px-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
          onClick={() => setExpanded((value) => !value)}
          type="button"
        >
          <span
            aria-hidden="true"
            className={cn(
              "size-3 shrink-0 rounded-full",
              started ? "bg-primary" : "bg-muted-foreground",
              blocked ? "bg-destructive" : "",
              micHot ? "motion-safe:animate-pulse" : "",
            )}
          />
          {expanded ? (
            <span className="min-w-0 flex-1 truncate font-medium">
              {liveStatusLabel(view.status)} · {liveRouteLabel(view.route)}
            </span>
          ) : null}
          {micHot ? (
            <Mic aria-hidden="true" className="size-4 shrink-0" />
          ) : (
            <MicOff aria-hidden="true" className="size-4 shrink-0" />
          )}
        </button>
        <span className="sr-only" role="status" aria-live="polite">
          {liveStatusLabel(view.status)}. {view.error ?? view.partialText ?? view.finalText ?? liveRouteLabel(view.route)}
        </span>

        {expanded ? (
          <div className="mt-2 grid gap-2 px-1 pb-1">
            <div className="h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                className="h-full rounded-full bg-primary transition-[width] duration-100 motion-reduce:transition-none"
                style={{ width: `${Math.round(Math.max(0, Math.min(1, view.level ?? 0)) * 100)}%` }}
              />
            </div>
            <p className="min-h-5 truncate text-sm text-muted-foreground">
              {view.error ?? view.partialText ?? view.finalText ?? view.inputDeviceLabel ?? view.hotkey}
            </p>
            <div className="flex items-center justify-between gap-2">
              <Button onClick={started ? onStop : onStart} size="sm" type="button" variant={started ? "outline" : "default"}>
                {started ? <Square /> : <Mic />}
                {started ? "Stop" : "Start"}
              </Button>
              <div className="flex gap-1">
                {onSave && view.finalText ? (
                  <Button aria-label="Save live session" onClick={onSave} size="icon-sm" type="button" variant="ghost">
                    <Save />
                  </Button>
                ) : null}
                {onOpenSettings ? (
                  <Button aria-label="Open live settings" onClick={onOpenSettings} size="icon-sm" type="button" variant="ghost">
                    <Settings />
                  </Button>
                ) : null}
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
