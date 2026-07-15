import { Check } from "@phosphor-icons/react/Check";
import { WarningCircle as CircleAlert } from "@phosphor-icons/react/WarningCircle";
import { ChatText as MessageSquareText } from "@phosphor-icons/react/ChatText";
import { Microphone as Mic } from "@phosphor-icons/react/Microphone";
import { ArrowCounterClockwise as RotateCcw } from "@phosphor-icons/react/ArrowCounterClockwise";
import { Sparkle as Sparkles } from "@phosphor-icons/react/Sparkle";
import { X } from "@phosphor-icons/react/X";
import gsap from "gsap";
import { useLayoutEffect, useRef, type CSSProperties, type ReactNode } from "react";

import { type OverlayModel, type OverlaySurface } from "@/components/live/live-overlay-state";
import { LiveWaveform } from "@/components/live/live-waveform";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export function LiveOverlayContent({
  model,
  onOpenScratch,
  onOpenTransform,
  onRetry,
  onStart,
  onStop,
  prefersReducedMotion,
  surface,
}: {
  model: OverlayModel;
  onOpenScratch?: () => void;
  onOpenTransform?: () => void;
  onRetry?: () => void;
  onStart?: () => void;
  onStop?: () => void;
  prefersReducedMotion: boolean;
  surface: OverlaySurface;
}) {
  if (surface === "collapsed") return <CollapsedOverlayView />;
  if (surface === "expanded") {
    return (
      <ExpandedOverlayView
        onOpenScratch={onOpenScratch}
        onOpenTransform={onOpenTransform}
        onStart={onStart}
      />
    );
  }
  if (surface === "success") return <SuccessOverlayView />;
  return (
    <RecordingOverlayView
      model={model}
      onRetry={onRetry}
      onStop={onStop}
      prefersReducedMotion={prefersReducedMotion}
    />
  );
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
  onRetry,
  onStop,
  prefersReducedMotion,
}: {
  model: OverlayModel;
  onRetry?: () => void;
  onStop?: () => void;
  prefersReducedMotion: boolean;
}) {
  const showsLiveRecordingContent = model.phase === "recording";
  const showsStopButton = showsLiveRecordingContent && model.recordingTriggerMode === "toggle";

  if (model.phase === "feedback" && model.errorMessage) {
    return <ErrorOverlayView message={model.errorMessage} onRetry={onRetry} />;
  }
  if (model.phase === "feedback") return <FailureIndicatorView onRetry={onRetry} />;

  return (
    <div className="relative grid h-full w-full place-items-center px-3" data-testid="live-recording-layout">
      <div className="absolute inset-0 grid place-items-center transition-opacity duration-200 ease-out">
        {model.phase === "initializing" ? (
          <InitializingDotsView prefersReducedMotion={prefersReducedMotion} />
        ) : showsLiveRecordingContent ? (
          <LiveWaveform audioLevel={model.audioLevel} prefersReducedMotion={prefersReducedMotion} showsActivityPulse />
        ) : (
          <ProcessingWaveformView />
        )}
      </div>

      {showsStopButton ? (
        <div className="absolute inset-0 flex items-center justify-end px-2" data-testid="live-toggle-actions">
          <FreeFlowIconButton label="Finish recording" onClick={onStop} tone="confirm">
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

function IslandInlineButton({ children, label, onClick }: ActionButtonProps) {
  return (
    <Button
      aria-label={label}
      className="size-8 rounded-full bg-white/10 p-0 text-white transition-colors hover:bg-white/20 hover:text-fuchsia-100 focus-visible:ring-white/60"
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

function FreeFlowIconButton({ children, label, onClick, tone = "cancel" }: ActionButtonProps & {
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
      title={label}
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

function ProcessingWaveformView() {
  return (
    <div className="live-processing-waveform flex h-5 items-center justify-center gap-1">
      {Array.from({ length: 5 }, (_, index) => (
        <span
          className="live-processing-pill w-1 rounded-full bg-white"
          key={index}
          style={{
            "--live-wave-delay": `${index * 110}ms`,
            height: 4 + (18 - 4) * processingAmplitude(index),
            opacity: 0.72,
          } as CSSProperties}
        />
      ))}
    </div>
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
        <span className="size-[4.5px] rounded-full bg-white opacity-25" key={index} />
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

function FreeFlowNeutralButton({ children, label, onClick }: ActionButtonProps) {
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

type ActionButtonProps = {
  children: ReactNode;
  label: string;
  onClick?: () => void;
};
