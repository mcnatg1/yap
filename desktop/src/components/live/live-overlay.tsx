import { Mic, MicOff, Save, Settings, Square } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import {
  currentMonitor,
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
} from "@tauri-apps/api/window";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
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
const compactWindow = { width: 96, height: 44 };
const controlsWindow = { width: 180, height: 86 };

export function LiveOverlay({ onOpenSettings, onSave, onStart, onStop, view }: LiveOverlayProps) {
  const [hovered, setHovered] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const windowModeRef = useRef<"compact" | "controls" | null>(null);
  const started = startedStatuses.has(view.status);
  const micHot = micHotStatuses.has(view.status);
  const blocked = view.status === "blocked";
  const finalText = view.finalText?.trim();
  const partialText = view.partialText?.trim();
  const transcriptText = [finalText, partialText].filter(Boolean).join(" ");
  const statusText = view.error ?? (transcriptText || liveRouteLabel(view.route));

  useEffect(() => {
    const node = rootRef.current;
    if (!node || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    let cancelled = false;
    void import("gsap").then(({ gsap }) => {
      if (cancelled) return;
      gsap.fromTo(
        node,
        { scaleX: started ? 0.82 : 1.08, scaleY: started ? 0.72 : 1.16 },
        { scaleX: 1, scaleY: 1, duration: 0.16, ease: "power2.out" },
      );
    });

    return () => {
      cancelled = true;
    };
  }, [started, view.status]);

  useEffect(() => {
    if (view.visibility === "hidden") return;

    const nextMode = hovered ? "controls" : "compact";
    if (windowModeRef.current === nextMode) return;
    windowModeRef.current = nextMode;

    void resizeOverlayWindow(nextMode === "controls");
  }, [hovered, view.visibility]);

  if (view.visibility === "hidden") return null;

  return (
    <div className="live-overlay-root pointer-events-none flex h-full items-start justify-center bg-transparent pt-2">
      <div
        ref={rootRef}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            setHovered(false);
          }
        }}
        onFocus={() => setHovered(true)}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        className={cn(
          "group/live pointer-events-auto flex flex-col items-center",
          hovered ? "z-10" : "",
        )}
      >
        <button
          aria-label={`${started ? "Stop" : "Start"} live. ${liveStatusLabel(view.status)}.`}
          className={cn(
            "hit-target flex items-center justify-center overflow-hidden rounded-full bg-neutral-950/90 shadow-[0_8px_24px_rgba(0,0,0,0.22)] ring-1 ring-white/55 outline-none transition-[width,height,background-color,box-shadow,transform] duration-150 ease-out focus-visible:ring-2 focus-visible:ring-white/80 active:scale-[0.96] motion-reduce:transition-none",
            started ? "h-4 w-[74px] px-2" : "h-2.5 w-[58px]",
            blocked ? "bg-destructive/90 ring-destructive/40" : "",
          )}
          onClick={started ? onStop : onStart}
          type="button"
        >
          {started ? <LiveWaveform level={view.level ?? 0} hot={micHot} /> : null}
        </button>
        <span className="sr-only" role="status" aria-live="polite">
          {liveStatusLabel(view.status)}. {statusText}
        </span>
        <div className="pointer-events-none mt-3 flex translate-y-1 scale-95 items-center gap-1.5 opacity-0 transition-[opacity,transform] duration-150 ease-out group-focus-within/live:pointer-events-auto group-focus-within/live:translate-y-0 group-focus-within/live:scale-100 group-focus-within/live:opacity-100 group-hover/live:pointer-events-auto group-hover/live:translate-y-0 group-hover/live:scale-100 group-hover/live:opacity-100 motion-reduce:transition-none">
          <LiveIconButton
            label={started ? "Stop live" : "Start live"}
            onClick={started ? onStop : onStart}
          >
            {started ? <Square /> : micHot ? <Mic /> : <MicOff />}
          </LiveIconButton>
          {onSave && view.finalText ? (
            <LiveIconButton label="Save live session" onClick={onSave}>
              <Save />
            </LiveIconButton>
          ) : null}
          {onOpenSettings ? (
            <LiveIconButton label="Open live settings" onClick={onOpenSettings}>
              <Settings />
            </LiveIconButton>
          ) : null}
        </div>
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

function LiveIconButton({
  children,
  label,
  onClick,
}: {
  children: ReactNode;
  label: string;
  onClick?: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={label}
          className="flex size-11 items-center justify-center rounded-full bg-neutral-950 text-white shadow-[0_10px_24px_rgba(0,0,0,0.26)] ring-1 ring-white/10 transition-[opacity,transform,background-color] duration-150 ease-out hover:bg-neutral-900 focus-visible:ring-2 focus-visible:ring-white/80 active:scale-[0.96] motion-reduce:transition-none [&_svg]:size-5"
          onClick={onClick}
          type="button"
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent side="top" sideOffset={8}>
        {label}
      </TooltipContent>
    </Tooltip>
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
