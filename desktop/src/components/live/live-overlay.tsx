import { invoke, isTauri } from "@tauri-apps/api/core";
import gsap from "gsap";
import { Check } from "@phosphor-icons/react/Check";
import { WarningCircle as CircleAlert } from "@phosphor-icons/react/WarningCircle";
import { ChatText as MessageSquareText } from "@phosphor-icons/react/ChatText";
import { Microphone as Mic } from "@phosphor-icons/react/Microphone";
import { ArrowCounterClockwise as RotateCcw } from "@phosphor-icons/react/ArrowCounterClockwise";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import { X } from "@phosphor-icons/react/X";
import type { CSSProperties, ReactNode } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  collapseGraceMs,
  modelFromLiveView,
  overlaySurface,
  previewOverlayFrame,
  successVisibleMs,
  type OverlayModel,
} from "@/components/live/live-overlay-state";
import { createNativeSurfaceSync } from "@/components/live/native-surface-sync";
import { type LiveSessionView } from "@/lib/app-types";
import { cn } from "@/lib/utils";

type LiveOverlayProps = {
  onOpenScratch?: () => void;
  onOpenTransform?: () => void;
  onRetry?: () => void;
  onStart?: () => void;
  onStop?: () => void;
  view: LiveSessionView;
};

export function LiveOverlay({
  onOpenScratch,
  onOpenTransform,
  onRetry,
  onStart,
  onStop,
  view,
}: LiveOverlayProps) {
  const model = modelFromLiveView(view);
  const [expanded, setExpanded] = useState(false);
  const [successVisible, setSuccessVisible] = useState(false);
  const native = isTauri();
  const prefersReducedMotion = usePrefersReducedMotion();
  const contentRef = useRef<HTMLDivElement>(null);
  const previousStatusRef = useRef(view.status);
  const collapseTimerRef = useRef<number | undefined>(undefined);
  const successTimerRef = useRef<number | undefined>(undefined);
  const hasCopyableFinal = Boolean(model.finalText?.trim());
  const surface = overlaySurface(model, expanded, successVisible && hasCopyableFinal);
  const previewFrame = native ? undefined : previewOverlayFrame(surface);
  const rootFrameStyle: CSSProperties | undefined = previewFrame;
  const hiddenIdle = view.visibility === "hidden" && model.phase === "idle";

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
  }, [prefersReducedMotion, surface]);

  useEffect(() => {
    if (model.phase === "idle") return;
    cancelCollapse();
    setExpanded(false);
  }, [model.phase]);

  useLayoutEffect(() => {
    if (!hiddenIdle) return;
    cancelCollapse();
    setExpanded(false);
  }, [hiddenIdle]);

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
  }, [hasCopyableFinal, view.status]);

  useEffect(() => {
    if (hiddenIdle) return;
    setNativeOverlaySurface({ surface });
  }, [hiddenIdle, surface]);

  useEffect(() => {
    return () => {
      cancelCollapse();
      clearSuccessTimer();
    };
  }, []);

  function clearSuccessTimer() {
    if (successTimerRef.current === undefined) return;
    window.clearTimeout(successTimerRef.current);
    successTimerRef.current = undefined;
  }

  function cancelCollapse() {
    if (collapseTimerRef.current === undefined) return;
    window.clearTimeout(collapseTimerRef.current);
    collapseTimerRef.current = undefined;
  }

  function openIdleIsland() {
    cancelCollapse();
    setExpanded(true);
  }

  function scheduleIdleCollapse() {
    cancelCollapse();
    collapseTimerRef.current = window.setTimeout(() => {
      collapseTimerRef.current = undefined;
      setExpanded(false);
    }, collapseGraceMs);
  }

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
        key="active-island"
        style={{
          backgroundColor: "black",
          borderRadius: native ? undefined : 14,
          overflow: "hidden",
        }}
      >
        <div className="h-full w-full" ref={contentRef}>
          {surface === "collapsed" ? (
            <CollapsedOverlayView />
          ) : surface === "expanded" ? (
            <ExpandedOverlayView
              onOpenScratch={onOpenScratch}
              onOpenTransform={onOpenTransform}
              onStart={onStart}
            />
          ) : surface === "success" ? (
            <SuccessOverlayView />
          ) : (
            <RecordingOverlayView
              model={model}
              onRetryButtonPressed={onRetry}
              onStopButtonPressed={onStop}
              prefersReducedMotion={prefersReducedMotion}
            />
          )}
        </div>
      </div>
    </div>
  );
}

const setNativeOverlaySurface = createNativeSurfaceSync(async ({ surface }) => {
  if (!isTauri()) return;
  await invoke("set_live_overlay_surface", { surface });
});

const liveOverlayLevelEvent = "yap-live-overlay-level";

export function emitLiveOverlayLevel(level: number) {
  const normalized = Number.isFinite(level) ? Math.min(1, Math.max(0, level)) : 0;
  window.dispatchEvent(new CustomEvent(liveOverlayLevelEvent, { detail: normalized }));
}

function CollapsedOverlayView() {
  return (
    <div className="flex h-full w-full items-center justify-center gap-2 px-3" aria-label="Yap dictation island">
      <Mic className="size-4 text-fuchsia-200" weight="fill" />
      <span className="text-[12px] font-semibold leading-none text-white">Yap</span>
    </div>
  );
}

function ExpandedOverlayView({
  onOpenScratch,
  onOpenTransform,
  onStart,
}: {
  onOpenScratch?: () => void;
  onOpenTransform?: () => void;
  onStart?: () => void;
}) {
  return (
    <div className="flex h-full w-full flex-col">
      <div className="flex h-10 shrink-0 items-center justify-center gap-2 border-b border-white/10 px-3">
        <Mic className="size-4 text-fuchsia-200" weight="fill" />
        <span className="text-[12px] font-semibold leading-none text-white">Yap</span>
      </div>
      <div className="flex min-h-0 flex-1 items-center justify-center gap-2 px-3">
        <IslandInlineButton label="Start dictating" onClick={onStart}>
          <Mic className="size-[18px]" weight="bold" />
        </IslandInlineButton>
        <IslandInlineButton label="Open scratch" onClick={onOpenScratch}>
          <MessageSquareText className="size-4" weight="bold" />
        </IslandInlineButton>
        <IslandInlineButton label="Open transform" onClick={onOpenTransform}>
          <Sparkles className="size-4" weight="bold" />
        </IslandInlineButton>
      </div>
    </div>
  );
}

function RecordingOverlayView({
  model,
  onRetryButtonPressed,
  onStopButtonPressed,
  prefersReducedMotion,
}: {
  model: OverlayModel;
  onRetryButtonPressed?: () => void;
  onStopButtonPressed?: () => void;
  prefersReducedMotion: boolean;
}) {
  const showsLiveRecordingContent = model.phase === "recording";
  const showsStopButton = showsLiveRecordingContent && model.recordingTriggerMode === "toggle";

  if (model.phase === "feedback" && model.errorMessage) {
    return <ErrorOverlayView message={model.errorMessage} onRetry={onRetryButtonPressed} />;
  }
  if (model.phase === "feedback") return <FailureIndicatorView onRetry={onRetryButtonPressed} />;

  return (
    <div className="relative grid h-full w-full place-items-center px-3" data-testid="live-recording-layout">
      <div className="absolute inset-0 grid place-items-center transition-opacity duration-200 ease-out">
        {model.phase === "initializing" ? (
          <InitializingDotsView prefersReducedMotion={prefersReducedMotion} />
        ) : showsLiveRecordingContent ? (
          <WaveformView audioLevel={model.audioLevel} prefersReducedMotion={prefersReducedMotion} showsActivityPulse />
        ) : (
          <ProcessingIndicatorView />
        )}
      </div>

      {showsStopButton ? (
        <div className="absolute inset-0 flex items-center justify-end px-2" data-testid="live-toggle-actions">
          <FreeFlowIconButton label="Finish recording" onClick={onStopButtonPressed} tone="confirm">
            <Check className="size-3" weight="bold" />
          </FreeFlowIconButton>
        </div>
      ) : (
        <div className="absolute inset-0 flex items-center px-3">
          <div className="w-6" />
          <div className="min-w-0 flex-1" />
        </div>
      )}
    </div>
  );
}

function IslandInlineButton({
  children,
  label,
  onClick,
}: {
  children: ReactNode;
  label: string;
  onClick?: () => void;
}) {
  return (
    <Button
      aria-label={label}
      className="size-8 rounded-full bg-white/10 p-0 text-white transition-colors hover:bg-white/20 hover:text-fuchsia-100 focus-visible:ring-white/60"
      onClick={onClick}
      size="icon-tight"
      type="button"
      variant="ghost"
      title={label}
    >
      {children}
    </Button>
  );
}

function FreeFlowIconButton({
  children,
  label,
  onClick,
  tone = "cancel",
}: {
  children: ReactNode;
  label: string;
  onClick?: () => void;
  tone?: "cancel" | "confirm";
}) {
  return (
    <Button
      aria-label={label}
      className={cn(
        "size-5 rounded-full p-0 text-white shadow-[inset_0_0_0_1px_rgba(255,255,255,0.16)] focus-visible:ring-white/60",
        tone === "confirm"
          ? "bg-white text-black hover:bg-emerald-100 hover:text-black"
          : "bg-white/18 hover:bg-red-500/85 hover:text-white",
      )}
      onClick={onClick}
      size="icon-tight"
      type="button"
      variant="ghost"
      title={label}
    >
      {children}
    </Button>
  );
}

function WaveformView({
  audioLevel,
  prefersReducedMotion,
  showsActivityPulse,
}: {
  audioLevel: number;
  prefersReducedMotion: boolean;
  showsActivityPulse?: boolean;
}) {
  const waveformRef = useRef<HTMLDivElement>(null);
  const renderLevelRef = useRef<(level: number) => void>(() => undefined);

  useLayoutEffect(() => {
    const waveform = waveformRef.current;
    if (!waveform) return;
    const bars = Array.from(waveform.querySelectorAll<HTMLElement>("[data-live-waveform-bar]"));
    const activityFloor = showsActivityPulse && !prefersReducedMotion ? 0.08 : 0;
    const scaleSetters = prefersReducedMotion
      ? []
      : bars.map((bar) => gsap.quickTo(bar, "scaleY", { duration: 0.08, ease: "power2.out" }));

    const renderLevel = (level: number) => {
      const normalizedLevel = Number.isFinite(level) ? Math.min(1, Math.max(0, level)) : 0;
      bars.forEach((bar, index) => {
        const amplitude = barAmplitude(
          normalizedLevel,
          waveformMultipliers[index] ?? 0,
          index,
          activityFloor,
        );
        const scale = (2 + (22 - 2) * amplitude) / 22;
        if (prefersReducedMotion) {
          gsap.set(bar, { scaleY: scale });
        } else {
          scaleSetters[index]?.(scale);
        }
      });
    };
    renderLevelRef.current = renderLevel;
    renderLevel(audioLevel);

    const handleLevel = (event: Event) => {
      renderLevel((event as CustomEvent<number>).detail);
    };
    window.addEventListener(liveOverlayLevelEvent, handleLevel);
    return () => {
      window.removeEventListener(liveOverlayLevelEvent, handleLevel);
      renderLevelRef.current = () => undefined;
      gsap.killTweensOf(bars);
    };
  }, [prefersReducedMotion, showsActivityPulse]);

  useEffect(() => {
    renderLevelRef.current(audioLevel);
  }, [audioLevel]);

  return (
    <div
      aria-hidden="true"
      className="flex h-6 w-12 items-center justify-center gap-[2.5px]"
      data-testid="live-waveform"
      ref={waveformRef}
    >
      {waveformMultipliers.map((_, index) => (
        <WaveformBar index={index} key={index} />
      ))}
    </div>
  );
}

const waveformMultipliers = [0.35, 0.55, 0.75, 0.9, 1.0, 0.9, 0.75, 0.55, 0.35] as const;
const waveformCenterIndex = (waveformMultipliers.length - 1) / 2;

function WaveformBar({ index }: { index: number }) {
  return (
    <span
      className="live-waveform-bar h-[22px] w-[3px] rounded-full bg-white"
      data-live-waveform-bar
      style={{
        transform: `scaleY(${(2 + (22 - 2) * barAmplitude(0, waveformMultipliers[index] ?? 0, index)) / 22})`,
      } as CSSProperties}
    />
  );
}

function barAmplitude(level: number, multiplier: number, index: number, activityFloor = 0) {
  const baseAmplitude = Math.min(Math.max(level, 0) * multiplier, 1);
  if (!activityFloor) return baseAmplitude;
  const centerBoost = 1 - Math.abs(index - waveformCenterIndex) / waveformCenterIndex;
  return Math.max(baseAmplitude, activityFloor * (0.62 + centerBoost * 0.38));
}

function ProcessingIndicatorView() {
  return <ProcessingWaveformView />;
}

function ProcessingWaveformView() {
  return (
    <div className="live-processing-waveform flex h-5 items-center justify-center gap-1">
      {Array.from({ length: 5 }, (_, index) => (
        <ProcessingPill
          amplitude={processingAmplitude(index)}
          index={index}
          key={index}
          opacity={0.72}
        />
      ))}
    </div>
  );
}

function ProcessingPill({ amplitude, index, opacity }: { amplitude: number; index: number; opacity: number }) {
  return (
    <span
      className="live-processing-pill w-1 rounded-full bg-white"
      style={{
        "--live-wave-delay": `${index * 110}ms`,
        height: 4 + (18 - 4) * amplitude,
        opacity,
      } as CSSProperties}
    />
  );
}

function processingAmplitude(index: number) {
  const centerDistance = Math.abs(index - 2) / 2;
  return 0.24 + (1 - centerDistance) * 0.18;
}

function InitializingDotsView({ prefersReducedMotion }: { prefersReducedMotion: boolean }) {
  const dotsRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const dots = dotsRef.current?.querySelectorAll("span");
    if (!dots?.length || prefersReducedMotion) return;
    const timeline = gsap.timeline({ repeat: -1 });
    timeline
      .to(dots, { duration: 0.16, opacity: 0.9, scale: 1.12, stagger: 0.1 })
      .to(dots, { duration: 0.22, opacity: 0.25, scale: 1, stagger: 0.1 }, "-=0.08")
      .to({}, { duration: 0.12 });
    return () => {
      timeline.kill();
    };
  }, [prefersReducedMotion]);

  return (
    <div className="flex items-center justify-center gap-1" ref={dotsRef}>
      {Array.from({ length: 3 }, (_, index) => (
        <span
          className="size-[4.5px] rounded-full bg-white opacity-25"
          key={index}
        />
      ))}
    </div>
  );
}

function SuccessOverlayView() {
  return (
    <div className="flex h-full w-full items-center justify-center gap-2 px-3">
      <span className="grid size-5 place-items-center rounded-full bg-emerald-500/90">
        <Check className="size-3.5 text-black" weight="bold" />
      </span>
      <span className="text-[12px] font-semibold leading-none text-white">Saved</span>
    </div>
  );
}

function FailureIndicatorView({ onRetry }: { onRetry?: () => void }) {
  return (
    <div className="flex h-full w-full items-center justify-center gap-2">
      <span className="grid size-5 place-items-center rounded-full bg-red-600/90">
        <X className="size-3 text-white" weight="bold" />
      </span>
      <FreeFlowNeutralButton label="Retry dictation" onClick={onRetry}>
        <RotateCcw className="size-3.5" weight="bold" />
      </FreeFlowNeutralButton>
    </div>
  );
}

function ErrorOverlayView({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <div className="flex h-full w-full items-center justify-center gap-1.5 px-3">
      <CircleAlert className="size-[13px] shrink-0 fill-red-600/90 text-red-600/90" />
      <span className="min-w-0 truncate text-[12px] font-medium leading-none text-white">{message}</span>
      <FreeFlowNeutralButton label="Retry dictation" onClick={onRetry}>
        <RotateCcw className="size-3.5" weight="bold" />
      </FreeFlowNeutralButton>
    </div>
  );
}

function FreeFlowNeutralButton({
  children,
  label,
  onClick,
}: {
  children: ReactNode;
  label: string;
  onClick?: () => void;
}) {
  return (
    <Button
      aria-label={label}
      className="size-[22px] shrink-0 rounded-full bg-white/12 p-0 text-white hover:bg-white/20 hover:text-fuchsia-100 focus-visible:ring-white/60"
      onClick={onClick}
      size="icon-tight"
      title={label}
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

function usePrefersReducedMotion() {
  const [prefersReducedMotion, setPrefersReducedMotion] = useState(false);

  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    setPrefersReducedMotion(media.matches);

    function handleChange(event: MediaQueryListEvent) {
      setPrefersReducedMotion(event.matches);
    }

    media.addEventListener("change", handleChange);
    return () => media.removeEventListener("change", handleChange);
  }, []);

  return prefersReducedMotion;
}
