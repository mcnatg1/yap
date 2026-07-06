import { isTauri } from "@tauri-apps/api/core";
import { currentMonitor, getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { ArrowCircleDown as ArrowDownCircle } from "@phosphor-icons/react/ArrowCircleDown";
import { Check } from "@phosphor-icons/react/Check";
import { WarningCircle as CircleAlert } from "@phosphor-icons/react/WarningCircle";
import { Copy } from "@phosphor-icons/react/Copy";
import { ChatText as MessageSquareText } from "@phosphor-icons/react/ChatText";
import { Microphone as Mic } from "@phosphor-icons/react/Microphone";
import { PencilSimple as Pencil } from "@phosphor-icons/react/PencilSimple";
import { ArrowCounterClockwise as RotateCcw } from "@phosphor-icons/react/ArrowCounterClockwise";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import { X } from "@phosphor-icons/react/X";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  hoverSensorHeight,
  idleSensorWidth,
  modelFromLiveView,
  overlayFrame,
  overlaySurface,
  retractMs,
  successVisibleMs,
  type OverlayPhase,
  type OverlayModel,
  type OverlaySurface,
} from "@/components/live/live-overlay-state";
import { type LiveSessionView } from "@/lib/app-types";
import { cn } from "@/lib/utils";

type LiveOverlayProps = {
  onCopyLast?: () => void;
  onOpenScratch?: () => void;
  onOpenTransform?: () => void;
  onRetry?: () => void;
  onStart?: () => void;
  onStop?: () => void;
  view: LiveSessionView;
};

const defaultWidth = 92;

export function LiveOverlay({
  onCopyLast,
  onOpenScratch,
  onOpenTransform,
  onRetry,
  onStart,
  onStop,
  view,
}: LiveOverlayProps) {
  const model = modelFromLiveView(view);
  const [entered, setEntered] = useState(false);
  const [peeked, setPeeked] = useState(false);
  const [retracting, setRetracting] = useState(false);
  const [successVisible, setSuccessVisible] = useState(false);
  const [showInitializing, setShowInitializing] = useState(false);
  const lockedProcessingWidthRef = useRef<number | undefined>(undefined);
  const lastNonProcessingWidthRef = useRef(defaultWidth);
  const previousPhaseRef = useRef<OverlayPhase>("idle");
  const previousEntrySurfaceRef = useRef<OverlaySurface>("sensor");
  const previousStatusRef = useRef(view.status);
  const retractTimerRef = useRef<number | undefined>(undefined);
  const resizeRunningRef = useRef(false);
  const resizeTargetRef = useRef({ height: hoverSensorHeight, width: idleSensorWidth });
  const successTimerRef = useRef<number | undefined>(undefined);
  const hasCopyableFinal = Boolean(model.finalText?.trim());
  const surface = overlaySurface(model, peeked, retracting, successVisible && hasCopyableFinal);
  const frame = overlayFrame(surface, model);
  const width = model.phase === "processing"
    ? lockedProcessingWidthRef.current ?? lastNonProcessingWidthRef.current
    : frame.width;

  useEffect(() => {
    const previousPhase = previousPhaseRef.current;
    previousPhaseRef.current = model.phase;
    if (model.phase === "processing" && previousPhase !== "processing") {
      lockedProcessingWidthRef.current = lastNonProcessingWidthRef.current;
    } else if (model.phase !== "processing") {
      lockedProcessingWidthRef.current = undefined;
      if (model.phase !== "idle") {
        lastNonProcessingWidthRef.current = overlayFrame(model.phase, model).width;
      }
    }
  }, [model]);

  useEffect(() => {
    if (surface === "sensor" || (surface === "initializing" && !showInitializing)) {
      setEntered(false);
      return;
    }

    if (previousEntrySurfaceRef.current === "sensor") {
      setEntered(false);
      const frame = window.requestAnimationFrame(() => setEntered(true));
      previousEntrySurfaceRef.current = surface;
      return () => window.cancelAnimationFrame(frame);
    }

    previousEntrySurfaceRef.current = surface;
    setEntered(true);
  }, [showInitializing, surface]);

  useEffect(() => {
    if (model.phase !== "initializing") {
      setShowInitializing(false);
      return;
    }

    const timer = window.setTimeout(() => setShowInitializing(true), 200);
    return () => window.clearTimeout(timer);
  }, [model.phase]);

  useEffect(() => {
    if (model.phase === "idle") return;
    cancelRetract();
    setPeeked(false);
    setRetracting(false);
  }, [model.phase]);

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
    if (surface === "sensor") {
      previousEntrySurfaceRef.current = "sensor";
    }
    scheduleOverlayResize(width, frame.height);
  }, [frame.height, surface, width]);

  useEffect(() => {
    return () => {
      if (retractTimerRef.current !== undefined) {
        window.clearTimeout(retractTimerRef.current);
      }
      clearSuccessTimer();
    };
  }, []);

  function clearSuccessTimer() {
    if (successTimerRef.current === undefined) return;
    window.clearTimeout(successTimerRef.current);
    successTimerRef.current = undefined;
  }

  function scheduleOverlayResize(width: number, height: number) {
    resizeTargetRef.current = { height, width };
    if (!resizeRunningRef.current) {
      void flushOverlayResize();
    }
  }

  async function flushOverlayResize() {
    resizeRunningRef.current = true;
    try {
      while (true) {
        const target = resizeTargetRef.current;
        await resizeOverlayWindow(target.width, target.height);
        if (target === resizeTargetRef.current) break;
      }
    } finally {
      resizeRunningRef.current = false;
    }
  }

  function cancelRetract() {
    if (retractTimerRef.current === undefined) return;
    window.clearTimeout(retractTimerRef.current);
    retractTimerRef.current = undefined;
  }

  function openIdlePreview() {
    cancelRetract();
    setRetracting(false);
    setPeeked(true);
  }

  function closeIdlePreview() {
    cancelRetract();
    setPeeked(false);
    setEntered(false);
    setRetracting(true);
    retractTimerRef.current = window.setTimeout(() => {
      retractTimerRef.current = undefined;
      setRetracting(false);
    }, retractMs);
  }

  if (surface === "sensor") {
    return (
      <div
        className="live-overlay-root pointer-events-auto h-full w-full bg-transparent"
        onMouseEnter={openIdlePreview}
      />
    );
  }
  if (model.phase === "initializing" && !showInitializing) return null;

  return (
    <div className="live-overlay-root pointer-events-none h-full w-full overflow-hidden bg-transparent p-0">
      <div
        className="pointer-events-auto h-full w-full text-white"
        onMouseLeave={() => {
          if (surface !== "peek") return;
          closeIdlePreview();
        }}
        style={{
          backgroundColor: "black",
          borderBottomLeftRadius: 14,
          borderBottomRightRadius: 14,
          overflow: "hidden",
          transform: entered ? "translateY(0)" : "translateY(-100%)",
          transition: "transform 180ms cubic-bezier(0.16, 1, 0.3, 1)",
        }}
      >
        {surface === "peek" ? (
          <PeekOverlayView
            onOpenScratch={onOpenScratch}
            onOpenTransform={onOpenTransform}
            onStart={onStart}
          />
        ) : surface === "success" ? (
          <SuccessOverlayView onCopyLast={onCopyLast} />
        ) : (
          <RecordingOverlayView
            model={model}
            onRetryButtonPressed={onRetry}
            onStopButtonPressed={onStop}
          />
        )}
      </div>
    </div>
  );
}

async function resizeOverlayWindow(width: number, height: number) {
  if (!isTauri()) return;

  const window = getCurrentWindow();
  await window.setSize(new LogicalSize(width, height));

  const monitor = await currentMonitor();
  if (!monitor) return;

  const position = monitor.position.toLogical(monitor.scaleFactor);
  const size = monitor.size.toLogical(monitor.scaleFactor);
  await window.setPosition(new LogicalPosition(position.x + Math.max((size.width - width) / 2, 0), position.y));
}

function PeekOverlayView({
  onOpenScratch,
  onOpenTransform,
  onStart,
}: {
  onOpenScratch?: () => void;
  onOpenTransform?: () => void;
  onStart?: () => void;
}) {
  return (
    <div className="flex h-full w-full items-center justify-center gap-2 px-3">
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
  );
}

function RecordingOverlayView({
  model,
  onRetryButtonPressed,
  onStopButtonPressed,
}: {
  model: OverlayModel;
  onRetryButtonPressed?: () => void;
  onStopButtonPressed?: () => void;
}) {
  const showsLiveRecordingContent = model.phase === "recording";
  const showsStopButton = showsLiveRecordingContent && model.recordingTriggerMode === "toggle";

  if (model.phase === "feedback" && model.errorMessage) {
    return <ErrorOverlayView message={model.errorMessage} onRetry={onRetryButtonPressed} />;
  }
  if (model.phase === "feedback") return <FailureIndicatorView onRetry={onRetryButtonPressed} />;
  if (model.phase === "updateAvailable") return <UpdateAvailableOverlayView />;

  return (
    <div className="relative grid h-full w-full place-items-center px-3">
      <div className="absolute inset-0 grid place-items-center transition-opacity duration-200 ease-out">
        {model.phase === "initializing" ? (
          <InitializingDotsView />
        ) : showsLiveRecordingContent ? (
          <WaveformView audioLevel={model.audioLevel} showsActivityPulse />
        ) : (
          <ProcessingIndicatorView />
        )}
      </div>

      {showsStopButton ? (
        <div className="absolute inset-0 flex items-center justify-between px-2.5">
          <FreeFlowIconButton label="Cancel recording" onClick={onStopButtonPressed} tone="cancel">
            <X className="size-3" weight="bold" />
          </FreeFlowIconButton>
          <div className="h-px w-11" />
          <FreeFlowIconButton label="Finish recording" onClick={onStopButtonPressed} tone="confirm">
            <Check className="size-3" weight="bold" />
          </FreeFlowIconButton>
        </div>
      ) : (
        <div className="absolute inset-0 flex items-center px-3">
          <div className="grid h-full w-6 place-items-center">
            {model.isCommandMode ? <CommandModeIndicator /> : null}
          </div>
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
      size="icon"
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
        "size-[22px] rounded-full p-0 text-white shadow-[inset_0_0_0_1px_rgba(255,255,255,0.16)] focus-visible:ring-white/60",
        tone === "confirm"
          ? "bg-white text-black hover:bg-emerald-100 hover:text-black"
          : "bg-white/18 hover:bg-red-500/85 hover:text-white",
      )}
      onClick={onClick}
      size="icon-xs"
      type="button"
      variant="ghost"
      title={label}
    >
      {children}
    </Button>
  );
}

function WaveformView({ audioLevel, showsActivityPulse }: { audioLevel: number; showsActivityPulse?: boolean }) {
  const pulseTime = useAnimationTime(Boolean(showsActivityPulse));
  return (
    <div className="flex h-6 w-11 items-center justify-center gap-[2.5px]">
      {waveformMultipliers.map((multiplier, index) => (
        <WaveformBar
          amplitude={barAmplitude(audioLevel, multiplier, index, pulseTime)}
          delay={Math.abs(index - waveformCenterIndex) * 0.01}
          key={index}
          response={0.18 + (Math.abs(index - waveformCenterIndex) / waveformCenterIndex) * 0.06}
        />
      ))}
    </div>
  );
}

const waveformMultipliers = [0.35, 0.55, 0.75, 0.9, 1.0, 0.9, 0.75, 0.55, 0.35] as const;
const waveformCenterIndex = (waveformMultipliers.length - 1) / 2;

function WaveformBar({ amplitude, delay, response }: { amplitude: number; delay: number; response: number }) {
  return (
    <span
      className="w-[3px] rounded-full bg-white"
      style={{
        height: 2 + (22 - 2) * amplitude,
        transition: `height ${Math.min(response, 0.12)}s cubic-bezier(0.16, 1, 0.3, 1) ${delay}s`,
      }}
    />
  );
}

function barAmplitude(level: number, multiplier: number, index: number, pulseTime?: number) {
  const baseAmplitude = Math.min(Math.max(level, 0) * multiplier, 1);
  if (pulseTime === undefined) return baseAmplitude;

  const travelingWave = 0.5 + 0.5 * Math.sin(pulseTime * 6.2 - index * 0.78);
  const shimmer = 0.5 + 0.5 * Math.sin(pulseTime * 3.1 + index * 0.5);
  const pulse = travelingWave * 0.22 + shimmer * 0.06;
  const saturationRelief = baseAmplitude * (0.74 + pulse);
  const quietPulse = (1 - baseAmplitude) * (0.04 + pulse * 0.28);
  return Math.min(saturationRelief + quietPulse, 1);
}

function ProcessingIndicatorView() {
  return <ProcessingWaveformView />;
}

function ProcessingWaveformView() {
  const time = useAnimationTime(true) ?? 0;
  return (
    <div className="flex h-5 items-center justify-center gap-1">
      {Array.from({ length: 5 }, (_, index) => (
        <ProcessingPill
          amplitude={processingAmplitude(index, time)}
          key={index}
          opacity={0.42 + processingPulse(index, time) * 0.52}
        />
      ))}
    </div>
  );
}

function ProcessingPill({ amplitude, opacity }: { amplitude: number; opacity: number }) {
  return (
    <span
      className="w-1 rounded-full bg-white"
      style={{
        height: 4 + (18 - 4) * amplitude,
        opacity,
      }}
    />
  );
}

function processingAmplitude(index: number, time: number) {
  const centerDistance = Math.abs(index - 2) / 2;
  const baseline = 0.18 + (1 - centerDistance) * 0.1;
  return Math.min(baseline + processingPulse(index, time) * 0.68, 1);
}

function processingPulse(index: number, time: number) {
  const cycle = 1.05;
  const stagger = 0.11;
  const phase = ((time - index * stagger) % cycle) / cycle;
  const wave = 0.5 + 0.5 * Math.sin(phase * 2 * Math.PI - Math.PI / 2);
  return Math.pow(wave, 1.9);
}

function InitializingDotsView() {
  const [activeDot, setActiveDot] = useState(0);

  useEffect(() => {
    const timer = window.setInterval(() => setActiveDot((value) => (value + 1) % 3), 500);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <div className="flex items-center justify-center gap-1">
      {Array.from({ length: 3 }, (_, index) => (
        <span
          className={cn("size-[4.5px] rounded-full bg-white transition-opacity duration-[400ms]", activeDot === index ? "opacity-90" : "opacity-25")}
          key={index}
        />
      ))}
    </div>
  );
}

function CommandModeIndicator() {
  return <Pencil className="size-4 text-white/90" weight="bold" />;
}

function SuccessOverlayView({ onCopyLast }: { onCopyLast?: () => void }) {
  return (
    <div className="flex h-full w-full items-center justify-center gap-2 px-3">
      <span className="grid size-5 place-items-center rounded-full bg-emerald-500/90">
        <Check className="size-3.5 text-black" weight="bold" />
      </span>
      <span className="text-[12px] font-semibold leading-none text-white">Saved</span>
      <FreeFlowNeutralButton label="Copy last dictation" onClick={onCopyLast}>
        <Copy className="size-3.5" weight="bold" />
      </FreeFlowNeutralButton>
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
      size="icon-xs"
      title={label}
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

function UpdateAvailableOverlayView() {
  return (
    <button className="flex h-full w-full items-center justify-center gap-[7px] text-[11px] font-semibold text-white" type="button">
      <ArrowDownCircle className="size-[13px] fill-white text-white" />
      <span>Update Available</span>
    </button>
  );
}

function useAnimationTime(enabled: boolean) {
  const [time, setTime] = useState<number | undefined>(undefined);

  useEffect(() => {
    if (!enabled) {
      setTime(undefined);
      return;
    }

    let frame = 0;
    let previous = 0;
    const tick = (now: number) => {
      if (now - previous >= 1000 / 60) {
        previous = now;
        setTime(now / 1000);
      }
      frame = window.requestAnimationFrame(tick);
    };
    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, [enabled]);

  return time;
}
