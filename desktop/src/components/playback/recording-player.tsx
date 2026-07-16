import { isTauri } from "@tauri-apps/api/core";
import { FileAudio } from "@phosphor-icons/react/FileAudio";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { Pause } from "@phosphor-icons/react/Pause";
import { Play } from "@phosphor-icons/react/Play";
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";

import { recordingActivityLabel } from "@/components/playback/recording-status";
import { roundedMediaSecond, seekRatioFromBounds } from "@/components/playback/recording-seek";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import { formatElapsed } from "@/lib/display-format";
import {
  isRecordingActive,
  isRecordingFinished,
  type RecordingJobView,
} from "@/lib/recording-job";
import { cn } from "@/lib/utils";

export function RecordingPlayer({
  item,
  onOpen,
  onReveal,
  variant = "panel",
}: {
  item: RecordingJobView;
  onOpen: (path: string) => void;
  onReveal: (path: string) => void;
  variant?: "panel" | "modal";
}) {
  const displayedSecondRef = useRef(0);
  const dragPointerIdRef = useRef<number | undefined>(undefined);
  const audioRef = useRef<HTMLAudioElement>(null);
  const lightweightTrackRef = useRef<HTMLDivElement>(null);
  const statusId = useId();
  const errorId = useId();
  const [currentSeconds, setCurrentSeconds] = useState(0);
  const [progressSeconds, setProgressSeconds] = useState(0);
  const [nativeMetadata, setNativeMetadata] = useState<{
    durationSeconds: number;
    recordingSrc: string;
  }>();
  const [failed, setFailed] = useState(false);
  const [playing, setPlaying] = useState(false);
  const recordingPath = item.playbackPath;
  const sourcePath = item.sourcePath;
  const recordingSrc = useMemo(
    () => (isTauri() && recordingPath ? recordingPath : undefined),
    [recordingPath],
  );
  const durationSeconds = nativeMetadata && nativeMetadata.recordingSrc === recordingSrc
    ? nativeMetadata.durationSeconds
    : undefined;
  const waveformMode = durationSeconds === undefined ? "pending" : "lightweight";
  const recordingStatus = failed
    ? "Playback unavailable"
    : isRecordingFinished(item.status)
      ? "Transcript saved"
      : isRecordingActive(item.status)
        ? recordingActivityLabel(item.status)
        : item.status === "failed"
          ? "Transcription failed"
          : "Queued";
  const canSeek = !failed && durationSeconds !== undefined && durationSeconds > 0;

  useEffect(() => {
    setCurrentSeconds(0);
    setProgressSeconds(0);
    displayedSecondRef.current = 0;
    dragPointerIdRef.current = undefined;
    setNativeMetadata(undefined);
    setFailed(false);
    setPlaying(false);
  }, [recordingSrc]);

  if (!recordingPath || !recordingSrc || !sourcePath) return null;

  function setDisplaySeconds(seconds: number) {
    const wholeSeconds = Math.floor(seconds);
    if (displayedSecondRef.current === wholeSeconds) return;
    displayedSecondRef.current = wholeSeconds;
    setCurrentSeconds(wholeSeconds);
  }

  function syncMediaDuration() {
    const duration = audioRef.current?.duration;
    if (recordingSrc && duration && Number.isFinite(duration)) {
      setNativeMetadata({ durationSeconds: duration, recordingSrc });
      setFailed(false);
    }
  }

  function syncMediaTime() {
    const currentTime = audioRef.current?.currentTime;
    if (currentTime !== undefined && Number.isFinite(currentTime)) {
      setProgressSeconds(currentTime);
      setDisplaySeconds(currentTime);
    }
  }

  function seekToRatio(ratio: number) {
    const audio = audioRef.current;
    const duration = durationSeconds ?? audio?.duration;
    if (!duration || !Number.isFinite(duration)) return;

    const nextSeconds = Math.max(0, Math.min(duration, ratio * duration));
    if (audio) audio.currentTime = nextSeconds;
    setProgressSeconds(nextSeconds);
    setDisplaySeconds(nextSeconds);
  }

  function seekBy(deltaSeconds: number) {
    const audio = audioRef.current;
    const duration = durationSeconds ?? audio?.duration;
    const currentTime = audio?.currentTime;
    if (currentTime === undefined || !duration || !Number.isFinite(duration)) return;
    seekToRatio((currentTime + deltaSeconds) / duration);
  }

  function seekFromPointer(event: ReactPointerEvent<HTMLDivElement>) {
    if (!canSeek) return;
    const track = lightweightTrackRef.current;
    const ratio = seekRatioFromBounds(
      event.clientX,
      (track ?? event.currentTarget).getBoundingClientRect(),
    );
    if (ratio !== undefined) seekToRatio(ratio);
  }

  function finishPointerSeek(event: ReactPointerEvent<HTMLDivElement>) {
    if (dragPointerIdRef.current !== event.pointerId) return;
    dragPointerIdRef.current = undefined;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }

  function togglePlayback() {
    const audio = audioRef.current;
    if (!audio) {
      setFailed(true);
      return;
    }
    if (audio.paused) {
      void audio.play().catch(() => setFailed(true));
    } else {
      audio.pause();
    }
  }

  const lightweightProgress = durationSeconds
    ? Math.max(0, Math.min(100, (progressSeconds / durationSeconds) * 100))
    : 0;

  return (
    <section
      aria-label="Recording playback"
      className={cn(
        "grid gap-3 border-b",
        variant === "modal" ? "bg-background px-8 py-6" : "bg-muted/40 p-4 sm:p-5",
      )}
    >
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2">
          <FileAudio className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="truncate text-sm font-medium">{item.name}</span>
              <Badge variant="secondary">
                {durationSeconds === undefined
                  ? "Local file"
                  : formatElapsed(Math.floor(durationSeconds))}
              </Badge>
            </div>
            <p className="truncate text-xs text-muted-foreground" id={statusId} title={sourcePath}>
              {recordingStatus}
            </p>
          </div>
        </div>
        <ButtonGroup aria-label="Recording actions">
          <Button
            aria-label={`Open recording ${item.name}`}
            onClick={() => onOpen(sourcePath)}
            size="sm"
            type="button"
            variant="secondary"
          >
            <FileAudio data-icon="inline-start" />
            Open
          </Button>
          <Button
            aria-label={`Reveal recording ${item.name}`}
            onClick={() => onReveal(sourcePath)}
            size="sm"
            type="button"
            variant="ghost"
          >
            <FolderOpen data-icon="inline-start" />
            Reveal
          </Button>
        </ButtonGroup>
      </div>
      <div
        className={cn(
          "rounded-lg border bg-background p-3",
          variant === "modal"
            && "rounded-2xl border-0 bg-muted/35 p-5 shadow-[0_0_0_1px_rgba(0,0,0,0.04)]",
        )}
      >
        <div className="flex items-center gap-3">
          <Button
            aria-label={playing ? `Pause recording ${item.name}` : `Play recording ${item.name}`}
            disabled={failed}
            onClick={togglePlayback}
            size="icon-lg"
            type="button"
            variant="secondary"
          >
            <span className="relative size-4">
              <Play
                className={`absolute inset-0 translate-x-px transition-[opacity,filter,scale] duration-300 ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:translate-x-px motion-reduce:scale-100 motion-reduce:blur-0 motion-reduce:transition-none ${
                  playing ? "scale-[0.25] opacity-0 blur-[4px]" : "scale-100 opacity-100 blur-0"
                }`}
              />
              <Pause
                className={`absolute inset-0 transition-[opacity,filter,scale] duration-300 ease-[cubic-bezier(0.2,0,0,1)] motion-reduce:scale-100 motion-reduce:blur-0 motion-reduce:transition-none ${
                  playing ? "scale-100 opacity-100 blur-0" : "scale-[0.25] opacity-0 blur-[4px]"
                }`}
              />
            </span>
          </Button>
          <div
            aria-label={`Seek recording ${item.name}`}
            aria-disabled={!canSeek}
            aria-valuemax={roundedMediaSecond(durationSeconds)}
            aria-valuemin={0}
            aria-valuenow={Math.min(currentSeconds, roundedMediaSecond(durationSeconds))}
            aria-valuetext={`${formatElapsed(currentSeconds)}${durationSeconds === undefined ? "" : ` of ${formatElapsed(roundedMediaSecond(durationSeconds))}`}`}
            className="relative h-14 min-w-0 flex-1 cursor-pointer overflow-hidden rounded-md bg-muted/60 outline-none ring-offset-background transition-[background-color,box-shadow] duration-150 ease-out hover:bg-muted/80 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 aria-disabled:cursor-default aria-disabled:opacity-70 aria-disabled:hover:bg-muted/60 [&>div]:pointer-events-none"
            data-waveform-mode={waveformMode}
            onKeyDown={(event) => {
              if (!canSeek) return;
              if (event.key === "ArrowLeft" || event.key === "ArrowDown") {
                event.preventDefault();
                seekBy(-5);
              } else if (event.key === "ArrowRight" || event.key === "ArrowUp") {
                event.preventDefault();
                seekBy(5);
              } else if (event.key === "PageDown") {
                event.preventDefault();
                seekBy(-30);
              } else if (event.key === "PageUp") {
                event.preventDefault();
                seekBy(30);
              } else if (event.key === "Home") {
                event.preventDefault();
                seekToRatio(0);
              } else if (event.key === "End") {
                event.preventDefault();
                seekToRatio(1);
              }
            }}
            onPointerCancel={finishPointerSeek}
            onPointerDown={(event) => {
              if (!canSeek) return;
              dragPointerIdRef.current = event.pointerId;
              event.currentTarget.setPointerCapture(event.pointerId);
              seekFromPointer(event);
            }}
            onPointerMove={(event) => {
              if (dragPointerIdRef.current === event.pointerId) seekFromPointer(event);
            }}
            onPointerUp={(event) => {
              if (dragPointerIdRef.current === event.pointerId) seekFromPointer(event);
              finishPointerSeek(event);
            }}
            role="slider"
            tabIndex={canSeek ? 0 : -1}
          >
            <div
              className="pointer-events-none absolute inset-x-3 top-1/2 h-1 -translate-y-1/2 overflow-hidden rounded-full bg-foreground/10"
              data-testid="lightweight-seek-track"
              ref={lightweightTrackRef}
            >
              <div
                className="h-full rounded-full bg-primary transition-[width] duration-100 ease-linear motion-reduce:transition-none"
                style={{ width: waveformMode === "lightweight" ? `${lightweightProgress}%` : "0%" }}
              />
            </div>
          </div>
        </div>
        <div className="mt-2 flex items-center justify-between gap-3 text-xs text-muted-foreground">
          <span>{playing ? "Playing" : recordingStatus}</span>
          <span className="tabular-nums">
            {formatElapsed(currentSeconds)}
            {durationSeconds === undefined
              ? null
              : ` / ${formatElapsed(Math.floor(durationSeconds))}`}
          </span>
        </div>
      </div>
      {failed ? (
        <p className="text-sm text-muted-foreground" id={errorId}>
          This recording is unsupported, moved, or unavailable in the app. Open it from disk instead.
        </p>
      ) : null}
      <audio
        aria-hidden="true"
        className="hidden"
        onCanPlay={() => setFailed(false)}
        onDurationChange={syncMediaDuration}
        onEnded={() => {
          setPlaying(false);
          syncMediaTime();
        }}
        onError={() => setFailed(true)}
        onLoadedMetadata={syncMediaDuration}
        onPause={() => setPlaying(false)}
        onPlay={() => setPlaying(true)}
        onTimeUpdate={syncMediaTime}
        preload="metadata"
        ref={audioRef}
        src={recordingSrc}
      />
    </section>
  );
}
