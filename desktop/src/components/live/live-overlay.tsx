import { currentMonitor, getCurrentWindow, LogicalPosition, LogicalSize } from "@tauri-apps/api/window";
import { ArrowDownCircle, CircleAlert, LoaderCircle, Pencil, Square, X } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { type LiveCaptureMode, type LiveSessionView } from "@/lib/app-types";
import { cn } from "@/lib/utils";

type LiveOverlayProps = {
  onStop?: () => void;
  view: LiveSessionView;
};

type OverlayPhase = "idle" | "initializing" | "recording" | "transcribing" | "feedback" | "updateAvailable";

type OverlayModel = {
  audioLevel: number;
  errorMessage?: string;
  isCommandMode: boolean;
  phase: OverlayPhase;
  recordingTriggerMode: "hold" | "toggle";
  updateVersion?: string;
};

const dropDownHeight = 38;
const hoverSensorHeight = 4;
const retractMs = 180;
const defaultWidth = 92;
const toggleWidth = 150;
const commandModeWidth = 180;
const updateWidth = 190;
const minErrorWidth = 180;
const maxErrorWidth = 420;

export function LiveOverlay({ onStop, view }: LiveOverlayProps) {
  const model = modelFromLiveView(view);
  const [entered, setEntered] = useState(false);
  const [peeked, setPeeked] = useState(false);
  const [retracting, setRetracting] = useState(false);
  const [showInitializing, setShowInitializing] = useState(false);
  const lockedTranscribingWidthRef = useRef<number | undefined>(undefined);
  const lastNonTranscribingWidthRef = useRef(defaultWidth);
  const previousPhaseRef = useRef<OverlayPhase>("idle");
  const previousEntryPhaseRef = useRef<OverlayPhase>("idle");
  const retractTimerRef = useRef<number | undefined>(undefined);
  const idlePreviewOpen = model.phase === "idle" && (peeked || retracting);
  const visiblePhase = idlePreviewOpen ? "recording" : model.phase;
  const visibleModel = idlePreviewOpen ? { ...model, phase: "recording" as const } : model;
  const width = visiblePhase === "transcribing"
    ? lockedTranscribingWidthRef.current ?? lastNonTranscribingWidthRef.current
    : overlayWidth(visibleModel);

  useEffect(() => {
    const previousPhase = previousPhaseRef.current;
    previousPhaseRef.current = model.phase;
    if (model.phase === "transcribing" && previousPhase !== "transcribing") {
      lockedTranscribingWidthRef.current = lastNonTranscribingWidthRef.current;
    } else if (model.phase !== "transcribing") {
      lockedTranscribingWidthRef.current = undefined;
      if (model.phase !== "idle") {
        lastNonTranscribingWidthRef.current = overlayWidth(model);
      }
    }
  }, [model]);

  useEffect(() => {
    if ((model.phase === "idle" && !peeked) || (model.phase === "initializing" && !showInitializing)) {
      setEntered(false);
      return;
    }

    if (previousEntryPhaseRef.current === "idle") {
      setEntered(false);
      const frame = window.requestAnimationFrame(() => setEntered(true));
      previousEntryPhaseRef.current = model.phase;
      return () => window.cancelAnimationFrame(frame);
    }

    previousEntryPhaseRef.current = model.phase;
    setEntered(true);
  }, [model.phase, peeked, showInitializing]);

  useEffect(() => {
    if (model.phase !== "initializing") {
      setShowInitializing(false);
      return;
    }

    const timer = window.setTimeout(() => setShowInitializing(true), 200);
    return () => window.clearTimeout(timer);
  }, [model.phase]);

  useEffect(() => {
    if (model.phase === "idle" && !peeked && !retracting) {
      previousEntryPhaseRef.current = "idle";
      void resizeOverlayWindow(defaultWidth, hoverSensorHeight);
      return;
    }
    void resizeOverlayWindow(width, dropDownHeight);
  }, [model.phase, peeked, retracting, width]);

  useEffect(() => {
    return () => {
      if (retractTimerRef.current !== undefined) {
        window.clearTimeout(retractTimerRef.current);
      }
    };
  }, []);

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
    setRetracting(true);
    retractTimerRef.current = window.setTimeout(() => {
      retractTimerRef.current = undefined;
      setRetracting(false);
    }, retractMs);
  }

  if (model.phase === "idle" && !peeked && !retracting) {
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
        className="pointer-events-auto h-full w-full overflow-hidden bg-black text-white"
        onMouseLeave={() => {
          if (model.phase !== "idle") return;
          closeIdlePreview();
        }}
        style={{
          borderBottomLeftRadius: 12,
          borderBottomRightRadius: 12,
          transform: entered ? "translateY(0)" : "translateY(-100%)",
          transition: "transform 180ms cubic-bezier(0.16, 1, 0.3, 1)",
        }}
      >
        <RecordingOverlayView model={visibleModel} onStopButtonPressed={onStop} />
      </div>
    </div>
  );
}

function modelFromLiveView(view: LiveSessionView): OverlayModel {
  const triggerMode = triggerModeFromCaptureMode(view.captureMode);
  if (view.visibility === "hidden" || view.status === "idle") {
    return { audioLevel: 0, isCommandMode: false, phase: "idle", recordingTriggerMode: triggerMode };
  }

  switch (view.status) {
    case "armed":
      return { audioLevel: 0, isCommandMode: false, phase: "initializing", recordingTriggerMode: triggerMode };
    case "listening":
    case "speaking":
      return { audioLevel: view.level ?? 0, isCommandMode: false, phase: "recording", recordingTriggerMode: triggerMode };
    case "settling":
    case "saving":
      return { audioLevel: 0, isCommandMode: false, phase: "transcribing", recordingTriggerMode: triggerMode };
    case "blocked":
      return {
        audioLevel: 0,
        errorMessage: view.error,
        isCommandMode: false,
        phase: "feedback",
        recordingTriggerMode: triggerMode,
      };
  }
}

function triggerModeFromCaptureMode(captureMode: LiveCaptureMode): "hold" | "toggle" {
  return captureMode === "toggle" ? "toggle" : "hold";
}

function overlayWidth(model: OverlayModel, lockedWidth?: number) {
  if (lockedWidth && model.phase === "transcribing") return lockedWidth;
  if (model.phase === "feedback") {
    if (!model.errorMessage) return defaultWidth;
    return Math.min(maxErrorWidth, Math.max(minErrorWidth, model.errorMessage.length * 6.8 + 60));
  }
  if (model.phase === "updateAvailable") return updateWidth;
  if (model.isCommandMode) return commandModeWidth;
  if (model.phase === "recording" && model.recordingTriggerMode === "toggle") return toggleWidth;
  return defaultWidth;
}

async function resizeOverlayWindow(width: number, height: number) {
  const window = getCurrentWindow();
  await window.setSize(new LogicalSize(width, height));

  const monitor = await currentMonitor();
  if (!monitor) return;

  const position = monitor.position.toLogical(monitor.scaleFactor);
  const size = monitor.size.toLogical(monitor.scaleFactor);
  await window.setPosition(new LogicalPosition(position.x + Math.max((size.width - width) / 2, 0), position.y));
}

function RecordingOverlayView({
  model,
  onStopButtonPressed,
}: {
  model: OverlayModel;
  onStopButtonPressed?: () => void;
}) {
  const showsLiveRecordingContent = model.phase === "recording";
  const showsStopButton = showsLiveRecordingContent && model.recordingTriggerMode === "toggle";

  if (model.phase === "feedback" && model.errorMessage) {
    return <ErrorOverlayView message={model.errorMessage} />;
  }
  if (model.phase === "feedback") return <FailureIndicatorView />;
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

      <div className="absolute inset-0 flex items-center px-3">
        <div className="grid h-full w-6 place-items-center">
          {model.isCommandMode ? <CommandModeIndicator /> : null}
        </div>
        <div className="min-w-0 flex-1" />
        <div className="grid h-full w-8 place-items-end items-center">
          {showsStopButton ? (
            <FreeFlowIconButton label="Stop recording" onClick={onStopButtonPressed}>
              <Square className="size-[7px] fill-white" strokeWidth={0} />
            </FreeFlowIconButton>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function FreeFlowIconButton({
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
      className="size-[14px] rounded-full bg-red-600/90 p-0 text-white hover:bg-red-600 hover:text-white focus-visible:ring-white/60"
      onClick={onClick}
      size="icon-xs"
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

function WaveformView({ audioLevel, showsActivityPulse }: { audioLevel: number; showsActivityPulse?: boolean }) {
  const pulseTime = useAnimationTime(Boolean(showsActivityPulse));
  return (
    <div className="flex h-6 items-center justify-center gap-[2.5px]">
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
        transition: `height ${response}s cubic-bezier(0.16, 1, 0.3, 1) ${delay}s`,
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
  const [showsExtendedSpinner, setShowsExtendedSpinner] = useState(false);

  useEffect(() => {
    setShowsExtendedSpinner(false);
    const timer = window.setTimeout(() => setShowsExtendedSpinner(true), 1000);
    return () => window.clearTimeout(timer);
  }, []);

  return showsExtendedSpinner ? (
    <LoaderCircle className="size-4 animate-spin text-white" strokeWidth={2.5} />
  ) : (
    <ProcessingWaveformView />
  );
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
  return <Pencil className="size-4 text-white/90" strokeWidth={2.4} />;
}

function FailureIndicatorView() {
  return (
    <div className="grid h-full w-full place-items-center">
      <span className="grid size-5 place-items-center rounded-full bg-red-600/90">
        <X className="size-3 text-white" strokeWidth={3} />
      </span>
    </div>
  );
}

function ErrorOverlayView({ message }: { message: string }) {
  return (
    <div className="flex h-full w-full items-center justify-center gap-1.5 px-3">
      <CircleAlert className="size-[13px] shrink-0 fill-red-600/90 text-red-600/90" />
      <span className="min-w-0 truncate text-[12px] font-medium leading-none text-white">{message}</span>
    </div>
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
      if (now - previous >= 1000 / 30) {
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
