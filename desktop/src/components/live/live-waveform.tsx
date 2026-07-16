import gsap from "gsap";
import { useEffect, useLayoutEffect, useRef, type CSSProperties } from "react";

const liveOverlayLevelEvent = "yap-live-overlay-level";
const waveformMultipliers = [0.35, 0.55, 0.75, 0.9, 1.0, 0.9, 0.75, 0.55, 0.35] as const;
const waveformCenterIndex = (waveformMultipliers.length - 1) / 2;

export function emitLiveOverlayLevel(level: number) {
  const normalized = Number.isFinite(level) ? Math.min(1, Math.max(0, level)) : 0;
  window.dispatchEvent(new CustomEvent(liveOverlayLevelEvent, { detail: normalized }));
}

export function LiveWaveform({
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
