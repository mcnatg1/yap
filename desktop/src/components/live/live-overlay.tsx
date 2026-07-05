import { currentMonitor, getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { Check, X } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";

import { liveRouteLabel, liveStatusLabel, type LiveSessionView } from "@/lib/app-types";
import { cn } from "@/lib/utils";

type LiveOverlayProps = {
  onHide?: () => void;
  onStart?: () => void;
  onStop?: () => void;
  view: LiveSessionView;
};

const startedStatuses = new Set<LiveSessionView["status"]>(["armed", "listening", "speaking", "settling"]);
const micHotStatuses = new Set<LiveSessionView["status"]>(["listening", "speaking", "settling"]);
const compactWindow = { width: 96, height: 44 };
const controlsWindow = { width: 120, height: 56 };

export function LiveOverlay({ onHide, onStart, onStop, view }: LiveOverlayProps) {
  const [hovered, setHovered] = useState(false);
  const [locked, setLocked] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const windowModeRef = useRef<"compact" | "controls" | null>(null);
  const started = startedStatuses.has(view.status);
  const micHot = micHotStatuses.has(view.status);
  const controlsOpen = hovered || locked;
  const statusText = view.error ?? ([view.finalText, view.partialText].filter(Boolean).join(" ") || liveRouteLabel(view.route));

  useEffect(() => {
    if (view.visibility === "hidden") setLocked(false);
  }, [view.visibility]);

  useEffect(() => {
    const node = rootRef.current;
    if (!node || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let cancelled = false;
    void import("gsap").then(({ gsap }) => {
      if (cancelled) return;
      gsap.fromTo(
        node,
        { scaleX: controlsOpen ? 0.78 : 1.08, scaleY: controlsOpen ? 0.74 : 1.1 },
        { scaleX: 1, scaleY: 1, duration: 0.14, ease: "power2.out" },
      );
    });

    return () => {
      cancelled = true;
    };
  }, [controlsOpen, started, view.status]);

  useEffect(() => {
    if (view.visibility === "hidden") return;

    const nextMode = controlsOpen ? "controls" : "compact";
    if (windowModeRef.current === nextMode) return;
    windowModeRef.current = nextMode;

    void resizeOverlayWindow(controlsOpen);
  }, [controlsOpen, view.visibility]);

  if (view.visibility === "hidden") return null;

  return (
    <div className="live-overlay-root pointer-events-none flex h-full items-center justify-center bg-transparent">
      <div
        className="pointer-events-auto"
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) setHovered(false);
        }}
        onDoubleClick={(event) => {
          event.preventDefault();
          setLocked((value) => !value);
        }}
        onFocus={() => setHovered(true)}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onContextMenu={(event) => {
          if (started) return;
          event.preventDefault();
          onHide?.();
        }}
        ref={rootRef}
      >
        {controlsOpen ? (
          <div
            aria-label={`${liveStatusLabel(view.status)}. ${statusText}`}
            className="flex h-10 w-[100px] items-center gap-1 rounded-full bg-neutral-950 p-1 text-white shadow-[0_8px_20px_rgba(0,0,0,0.24)] ring-1 ring-white/20 transition-[width,height,background-color,box-shadow] duration-150 ease-out motion-reduce:transition-none"
            role="group"
          >
            <OverlayIconButton label={started ? "Cancel live" : "Close live controls"} onClick={started ? onStop : () => setLocked(false)}>
              <X />
            </OverlayIconButton>
            <button
              aria-label={started ? "Live waveform" : "Start live"}
              className="flex h-8 min-w-0 flex-1 items-center justify-center rounded-full text-white/85 outline-none transition-[color,opacity] duration-150 hover:text-white focus-visible:ring-2 focus-visible:ring-white/55 active:scale-[0.96] motion-reduce:transition-none"
              onClick={(event) => {
                if (event.detail > 1) return;
                if (!started) onStart?.();
              }}
              type="button"
            >
              <LiveDots hot={micHot} level={view.level ?? 0} />
            </button>
            <OverlayIconButton label={started ? "Finish live" : "Start live"} onClick={started ? onStop : onStart} variant="light">
              <Check />
            </OverlayIconButton>
          </div>
        ) : (
          <button
            aria-label={`${started ? "Stop" : "Start"} live. ${liveStatusLabel(view.status)}.`}
            className={cn(
              "hit-target flex items-center justify-center overflow-hidden rounded-full bg-neutral-950/90 shadow-[0_8px_20px_rgba(0,0,0,0.22)] ring-1 ring-white/50 outline-none transition-[width,height,background-color,box-shadow,transform] duration-150 ease-out focus-visible:ring-2 focus-visible:ring-white/80 active:scale-[0.96] motion-reduce:transition-none",
              started ? "h-4 w-[74px] px-2" : "h-2.5 w-[58px]",
              view.status === "blocked" ? "bg-destructive/90 ring-destructive/40" : "",
            )}
            onClick={(event) => {
              if (event.detail > 1) return;
              (started ? onStop : onStart)?.();
            }}
            type="button"
          >
            {started ? <LiveWaveform hot={micHot} level={view.level ?? 0} /> : null}
          </button>
        )}
        <span className="sr-only" role="status" aria-live="polite">
          {liveStatusLabel(view.status)}. {statusText}
        </span>
      </div>
    </div>
  );
}

async function resizeOverlayWindow(showControls: boolean) {
  const next = showControls ? controlsWindow : compactWindow;
  const window = getCurrentWindow();
  await window.setSize(new LogicalSize(next.width, next.height));

  const monitor = await currentMonitor();
  if (!monitor) return;

  const position = monitor.position.toLogical(monitor.scaleFactor);
  const size = monitor.size.toLogical(monitor.scaleFactor);
  await window.setPosition(
    new LogicalPosition(position.x + Math.max((size.width - next.width) / 2, 8), position.y + 8),
  );
}

function OverlayIconButton({
  children,
  label,
  onClick,
  variant = "dark",
}: {
  children: ReactNode;
  label: string;
  onClick?: () => void;
  variant?: "dark" | "light";
}) {
  return (
    <button
      aria-label={label}
      className={cn(
        "flex size-8 shrink-0 items-center justify-center rounded-full outline-none transition-[background-color,color,transform] duration-150 focus-visible:ring-2 focus-visible:ring-white/65 active:scale-[0.96] motion-reduce:transition-none [&_svg]:size-4",
        variant === "light"
          ? "bg-white text-neutral-950 hover:text-primary"
          : "bg-white/18 text-white hover:bg-white/24 hover:text-white/85",
      )}
      onClick={(event) => {
        if (event.detail > 1) return;
        onClick?.();
      }}
      type="button"
    >
      {children}
    </button>
  );
}

function LiveDots({ hot, level }: { hot: boolean; level: number }) {
  const opacity = hot ? 0.95 : 0.72 + Math.min(0.2, level * 0.2);

  return (
    <span aria-hidden="true" className="flex items-center justify-center gap-1">
      {Array.from({ length: 8 }).map((_, index) => (
        <span className="size-0.5 rounded-full bg-current" key={index} style={{ opacity }} />
      ))}
    </span>
  );
}

function LiveWaveform({ hot, level }: { hot: boolean; level: number }) {
  const clamped = Math.max(0.05, Math.min(1, level));
  const bars = [0.35, 0.7, 0.48, 0.9, 0.42, 0.68, 0.32];

  return (
    <span aria-hidden="true" className="flex h-3 w-full items-center justify-center gap-1">
      {bars.map((bar, index) => (
        <span
          className={cn(
            "w-0.5 rounded-full bg-white/90 transition-[height,opacity] duration-75 motion-reduce:transition-none",
            hot ? "opacity-100" : "opacity-55",
          )}
          key={index}
          style={{ height: `${Math.max(3, Math.round((bar * clamped + 0.18) * 12))}px` }}
        />
      ))}
    </span>
  );
}
