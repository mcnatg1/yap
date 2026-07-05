import { currentMonitor, getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import gsap from "gsap";
import { Check, X } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import WaveSurfer from "wavesurfer.js";

import { Button } from "@/components/ui/button";
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
const overlaySizes = {
  idle: { pill: { width: 58, height: 10 }, window: { width: 64, height: 18 } },
  active: { pill: { width: 74, height: 16 }, window: { width: 80, height: 24 } },
  controls: { pill: { width: 100, height: 40 }, window: { width: 104, height: 44 } },
} as const;
type OverlayMode = keyof typeof overlaySizes;

export function LiveOverlay({ onHide, onStart, onStop, view }: LiveOverlayProps) {
  const [hovered, setHovered] = useState(false);
  const [locked, setLocked] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const pillRef = useRef<HTMLDivElement>(null);
  const compactRef = useRef<HTMLButtonElement>(null);
  const controlsRef = useRef<HTMLDivElement>(null);
  const windowModeRef = useRef<OverlayMode | null>(null);
  const started = startedStatuses.has(view.status);
  const micHot = micHotStatuses.has(view.status);
  const controlsOpen = hovered || locked;
  const mode: OverlayMode = controlsOpen ? "controls" : started ? "active" : "idle";
  const statusText = view.error ?? ([view.finalText, view.partialText].filter(Boolean).join(" ") || liveRouteLabel(view.route));

  useEffect(() => {
    if (view.visibility === "hidden") setLocked(false);
  }, [view.visibility]);

  useLayoutEffect(() => {
    const pill = pillRef.current;
    const compact = compactRef.current;
    const controls = controlsRef.current;
    if (!pill || !compact || !controls) return;

    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const size = overlaySizes[mode].pill;
    const duration = reduce ? 0 : 0.18;
    const targets = [pill, compact, controls];

    gsap.killTweensOf(targets);
    gsap.to(pill, {
      width: size.width,
      height: size.height,
      duration,
      ease: "power3.out",
      overwrite: "auto",
    });
    gsap.to(compact, {
      autoAlpha: controlsOpen ? 0 : 1,
      scale: controlsOpen ? 0.92 : 1,
      duration: reduce ? 0 : 0.1,
      ease: "power2.out",
      overwrite: "auto",
    });
    gsap.to(controls, {
      autoAlpha: controlsOpen ? 1 : 0,
      scale: controlsOpen ? 1 : 0.94,
      duration: reduce ? 0 : 0.12,
      ease: "power2.out",
      overwrite: "auto",
    });

    return () => {
      gsap.killTweensOf(targets);
    };
  }, [controlsOpen, mode]);

  useEffect(() => {
    if (view.visibility === "hidden") return;

    const previousMode = windowModeRef.current;
    if (windowModeRef.current === mode) return;
    windowModeRef.current = mode;

    const previousArea = previousMode ? overlaySizes[previousMode].window.width * overlaySizes[previousMode].window.height : 0;
    const nextArea = overlaySizes[mode].window.width * overlaySizes[mode].window.height;
    const delay = previousArea > nextArea ? 120 : 0;
    const resizeTimer = window.setTimeout(() => {
      void resizeOverlayWindow(mode);
    }, delay);

    return () => window.clearTimeout(resizeTimer);
  }, [mode, view.visibility]);

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
        <div
          aria-label={`${liveStatusLabel(view.status)}. ${statusText}`}
          className={cn(
            "relative overflow-hidden rounded-full bg-neutral-950/90 text-white shadow-[0_8px_20px_rgba(0,0,0,0.22)] ring-1 ring-white/35",
            view.status === "blocked" ? "bg-destructive/90 ring-destructive/40" : "",
          )}
          ref={pillRef}
          role={controlsOpen ? "group" : undefined}
          style={{
            height: overlaySizes.idle.pill.height,
            width: overlaySizes.idle.pill.width,
          }}
        >
          <Button
            aria-hidden={controlsOpen}
            aria-label={`${started ? "Stop" : "Start"} live. ${liveStatusLabel(view.status)}.`}
            className={cn(
              "absolute inset-0 h-full w-full overflow-hidden rounded-full bg-transparent p-0 text-white hover:bg-transparent hover:text-white focus-visible:ring-white/80",
              controlsOpen ? "pointer-events-none" : "pointer-events-auto",
            )}
            onClick={(event) => {
              if (event.detail > 1) return;
              (started ? onStop : onStart)?.();
            }}
            ref={compactRef}
            size="sm"
            tabIndex={controlsOpen ? -1 : 0}
            type="button"
            variant="ghost"
          >
            {started ? <LiveWaveform hot={micHot} level={view.level ?? 0} /> : null}
          </Button>
          <div
            aria-hidden={!controlsOpen}
            className={cn(
              "invisible absolute inset-0 flex items-center gap-1 p-1 opacity-0",
              controlsOpen ? "pointer-events-auto" : "pointer-events-none",
            )}
            ref={controlsRef}
          >
            <OverlayIconButton label={started ? "Cancel live" : "Close live controls"} onClick={started ? onStop : () => setLocked(false)} tabIndex={controlsOpen ? 0 : -1}>
              <X />
            </OverlayIconButton>
            <Button
              aria-label={started ? "Live waveform" : "Start live"}
              className="h-8 min-w-0 flex-1 rounded-full bg-transparent p-0 text-white/85 hover:bg-white/8 hover:text-white focus-visible:ring-white/55"
              onClick={(event) => {
                if (event.detail > 1) return;
                if (!started) onStart?.();
              }}
              size="sm"
              tabIndex={controlsOpen ? 0 : -1}
              type="button"
              variant="ghost"
            >
              <LiveDots hot={micHot} level={view.level ?? 0} />
            </Button>
            <OverlayIconButton label={started ? "Finish live" : "Start live"} onClick={started ? onStop : onStart} tabIndex={controlsOpen ? 0 : -1} variant="light">
              <Check />
            </OverlayIconButton>
          </div>
        </div>
        <span className="sr-only" role="status" aria-live="polite">
          {liveStatusLabel(view.status)}. {statusText}
        </span>
      </div>
    </div>
  );
}

async function resizeOverlayWindow(mode: OverlayMode) {
  const next = overlaySizes[mode].window;
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
  tabIndex,
  variant = "dark",
}: {
  children: ReactNode;
  label: string;
  onClick?: () => void;
  tabIndex?: number;
  variant?: "dark" | "light";
}) {
  return (
    <Button
      aria-label={label}
      className={cn(
        "size-8 shrink-0 rounded-full p-0 focus-visible:ring-white/65 [&_svg]:size-4",
        variant === "light"
          ? "bg-white text-neutral-950 hover:text-primary"
          : "bg-white/18 text-white hover:bg-white/24 hover:text-white/85",
      )}
      onClick={(event) => {
        if (event.detail > 1) return;
        onClick?.();
      }}
      size="icon-sm"
      tabIndex={tabIndex}
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

function LiveDots({ hot, level }: { hot: boolean; level: number }) {
  return <LiveLevelWaveform className="h-4 w-10" count={8} hot={hot} level={level} />;
}

function LiveWaveform({ hot, level }: { hot: boolean; level: number }) {
  return <LiveLevelWaveform className="h-3 w-full" count={10} hot={hot} level={level} />;
}

function LiveLevelWaveform({
  className,
  count,
  hot,
  level,
}: {
  className: string;
  count: number;
  hot: boolean;
  level: number;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const waveSurferRef = useRef<WaveSurfer | undefined>(undefined);
  const peaks = useLiveLevelPeaks(level, count);
  const color = hot ? "rgba(255, 255, 255, 0.96)" : "rgba(255, 255, 255, 0.58)";

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const waveSurfer = WaveSurfer.create({
      barGap: 2,
      barMinHeight: 2,
      barRadius: 999,
      barWidth: 2,
      container,
      cursorWidth: 0,
      duration: 1,
      height: "auto",
      hideScrollbar: true,
      interact: false,
      peaks: [signedPeaks(peaks)],
      progressColor: color,
      waveColor: color,
    });
    waveSurferRef.current = waveSurfer;

    return () => {
      waveSurfer.destroy();
      if (waveSurferRef.current === waveSurfer) waveSurferRef.current = undefined;
    };
  }, []);

  useEffect(() => {
    waveSurferRef.current?.setOptions({
      duration: 1,
      peaks: [signedPeaks(peaks)],
      progressColor: color,
      waveColor: color,
    });
  }, [color, peaks]);

  return <div aria-hidden="true" className={cn("overflow-hidden", className)} ref={containerRef} />;
}

function useLiveLevelPeaks(level: number, count: number) {
  const [peaks, setPeaks] = useState(() => Array.from({ length: count }, () => 0.2));
  const lastRef = useRef<number | undefined>(undefined);

  useEffect(() => {
    const clamped = Math.max(0, Math.min(1, level));
    if (lastRef.current === clamped) return;
    lastRef.current = clamped;

    setPeaks((current) => {
      const next = Math.max(0.16, Math.min(1, 0.16 + clamped * 0.84));
      return [...current.slice(1), next];
    });
  }, [level]);

  return peaks;
}

function signedPeaks(peaks: number[]) {
  return peaks.flatMap((peak) => [peak, -peak]);
}
