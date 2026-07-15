import gsap from "gsap";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { TranscriptPanel } from "@/components/panels/transcript-panel";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import type { RecordingJobView } from "@/lib/recording-job";

type MorphRect = {
  height: number;
  left: number;
  top: number;
  width: number;
};

type MorphPhase = "opening" | "closing";

function prefersReducedMotion() {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function dialogTargetRect(): MorphRect {
  const width = Math.min(1120, window.innerWidth - 40);
  const height = Math.min(720, window.innerHeight - 40);

  return {
    height,
    left: (window.innerWidth - width) / 2,
    top: (window.innerHeight - height) / 2,
    width,
  };
}

export function TranscriptReviewDialog({
  elapsedSeconds,
  item,
  morphOrigin,
  onCopy,
  onOpen,
  onOpenChange,
  onOpenHelp,
  onRetry,
  onReveal,
  open,
  running,
  text,
}: {
  elapsedSeconds: number;
  item?: RecordingJobView;
  morphOrigin?: MorphRect;
  onCopy: (item: RecordingJobView) => void;
  onOpen: (path: string) => void;
  onOpenChange: (open: boolean) => void;
  onOpenHelp?: () => void;
  onRetry: (id: string) => void;
  onReveal: (path: string) => void;
  open: boolean;
  running: boolean;
  text?: string;
}) {
  const morphLayerRef = useRef<HTMLDivElement>(null);
  const onOpenChangeRef = useRef(onOpenChange);
  const [phase, setPhase] = useState<MorphPhase | undefined>();

  useEffect(() => {
    onOpenChangeRef.current = onOpenChange;
  }, [onOpenChange]);

  useLayoutEffect(() => {
    if (!open) {
      setPhase(undefined);
      return;
    }

    setPhase(morphOrigin && !prefersReducedMotion() ? "opening" : undefined);
  }, [morphOrigin, open]);

  useEffect(() => {
    if (!phase || !morphOrigin) return;

    const layer = morphLayerRef.current;
    if (!layer) return;

    const target = dialogTargetRect();
    const start = phase === "opening" ? morphOrigin : target;
    const end = phase === "opening" ? target : morphOrigin;
    const duration = phase === "opening" ? 0.22 : 0.16;
    gsap.set(layer, {
      opacity: 1,
      scaleX: 1,
      scaleY: 1,
      transformOrigin: "top left",
      x: 0,
      y: 0,
    });
    const tween = gsap.to(layer, {
      duration,
      ease: phase === "opening" ? "power3.out" : "power2.inOut",
      opacity: phase === "closing" ? 0.45 : 1,
      scaleX: end.width / Math.max(1, start.width),
      scaleY: end.height / Math.max(1, start.height),
      x: end.left - start.left,
      y: end.top - start.top,
      onComplete: () => {
        if (phase === "opening") {
          setPhase(undefined);
        } else {
          setPhase(undefined);
          onOpenChangeRef.current(false);
        }
      },
    });

    return () => {
      tween.kill();
    };
  }, [morphOrigin, phase]);

  const morphStart = useMemo(() => {
    if (!phase || !morphOrigin) return undefined;
    return phase === "opening" ? morphOrigin : dialogTargetRect();
  }, [morphOrigin, phase]);
  const morphStyle: CSSProperties | undefined = morphStart
    ? {
        height: morphStart.height,
        left: morphStart.left,
        top: morphStart.top,
        width: morphStart.width,
      }
    : undefined;

  function handleOpenChange(nextOpen: boolean) {
    if (nextOpen) {
      onOpenChange(true);
      return;
    }

    if (phase === "closing") return;
    if (!morphOrigin || prefersReducedMotion()) {
      onOpenChange(false);
      return;
    }

    setPhase("closing");
  }

  return (
    <>
      <Dialog onOpenChange={handleOpenChange} open={open}>
        <DialogContent
          className={cn(
            "h-[min(720px,calc(100vh-40px))] w-[1120px] gap-0 overflow-hidden rounded-2xl border-0 bg-background p-0 shadow-[0_24px_80px_rgba(0,0,0,0.28)] !max-w-[calc(100vw-40px)] transition-opacity duration-100 data-[state=closed]:slide-out-to-bottom-1 data-[state=open]:slide-in-from-bottom-2 motion-reduce:animate-none",
            phase && "opacity-0",
          )}
          showCloseButton
        >
          <DialogHeader className="sr-only">
            <DialogTitle>{item?.name ?? "Recording review"}</DialogTitle>
            <DialogDescription>Recording playback and transcript review.</DialogDescription>
          </DialogHeader>
          {item ? (
            <TranscriptPanel
              elapsedSeconds={elapsedSeconds}
              item={item}
              onCopy={onCopy}
              onOpen={onOpen}
              onOpenHelp={onOpenHelp}
              onRetry={onRetry}
              onReveal={onReveal}
              running={running}
              text={text}
              variant="modal"
            />
          ) : null}
        </DialogContent>
      </Dialog>
      {phase && morphStyle ? (
        <div
          aria-hidden="true"
          className="fixed z-[60] rounded-2xl bg-background shadow-[0_24px_80px_rgba(0,0,0,0.24)] will-change-transform"
          ref={morphLayerRef}
          style={morphStyle}
        />
      ) : null}
    </>
  );
}
