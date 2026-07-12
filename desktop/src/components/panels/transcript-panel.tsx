import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { isTauri } from "@tauri-apps/api/core";
import { Copy } from "@phosphor-icons/react/Copy";
import { FileAudio } from "@phosphor-icons/react/FileAudio";
import { FileText } from "@phosphor-icons/react/FileText";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { Question as HelpCircle } from "@phosphor-icons/react/Question";
import { Pause } from "@phosphor-icons/react/Pause";
import { Play } from "@phosphor-icons/react/Play";
import { ArrowCounterClockwise as RotateCcw } from "@phosphor-icons/react/ArrowCounterClockwise";
import WaveSurfer from "wavesurfer.js";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import {
  formatElapsed,
  isRecordingActive,
  isRecordingFinished,
  queuedServerMessage,
  type RecordingJobStatus,
  type RecordingJobView,
} from "@/lib/app-types";
import { projectTranscriptText } from "@/lib/transcript-text";
import { cn } from "@/lib/utils";

function recordingActivityLabel(status: RecordingJobStatus) {
  switch (status) {
    case "uploading":
      return "Uploading";
    case "server_processing":
      return "Processing on server";
    case "diarization_running":
      return "Finding speakers";
    case "saving":
      return "Saving";
    default:
      return "Working";
  }
}

// WaveSurfer resamples into Float32 PCM. Unknown layouts use Web Audio's
// channel ceiling so admission fails toward the streaming native media path.
export const maxDecodedWaveformBytes = 32 * 1024 * 1024;
export const maxWaveformSourceBytes = 32 * 1024 * 1024;
export const decodedWaveformSampleRate = 8_000;
export const maxDecodedWaveformChannels = 32;
export const waveformReadyTimeoutMs = 10_000;
const decodedWaveformBytesPerSample = Float32Array.BYTES_PER_ELEMENT;

type DisposableWaveform = {
  destroy: () => void;
};

function canMountDecodedWaveform(byteLength: number, durationSeconds: number | undefined) {
  const decodedByteLength = durationSeconds === undefined
    ? Number.POSITIVE_INFINITY
    : Math.ceil(durationSeconds * decodedWaveformSampleRate) *
      maxDecodedWaveformChannels * decodedWaveformBytesPerSample;

  return (
    Number.isSafeInteger(byteLength) &&
    byteLength >= 0 &&
    byteLength <= maxWaveformSourceBytes &&
    durationSeconds !== undefined &&
    Number.isFinite(durationSeconds) &&
    durationSeconds > 0 &&
    Number.isSafeInteger(decodedByteLength) &&
    decodedByteLength <= maxDecodedWaveformBytes
  );
}

export function mountDecodedWaveform<T extends DisposableWaveform>({
  byteLength,
  create,
  durationSeconds,
  onReadyTimeout,
  readyTimeoutMs = waveformReadyTimeoutMs,
  requested,
  subscribe,
}: {
  byteLength: number;
  create: () => T;
  durationSeconds: number | undefined;
  onReadyTimeout?: () => void;
  readyTimeoutMs?: number;
  requested: boolean;
  subscribe: (waveform: T, lifecycle: {
    dispose: () => void;
    markReady: () => void;
  }) => Array<() => void>;
}) {
  if (!requested || !canMountDecodedWaveform(byteLength, durationSeconds)) {
    return undefined;
  }

  const waveform = create();
  let disposed = false;
  let ready = false;
  let readyTimer: ReturnType<typeof setTimeout> | undefined;
  let unsubscribers: Array<() => void> = [];
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    if (readyTimer !== undefined) clearTimeout(readyTimer);
    unsubscribers.forEach((unsubscribe) => unsubscribe());
    waveform.destroy();
  };
  const markReady = () => {
    if (disposed || ready) return;
    ready = true;
    if (readyTimer !== undefined) {
      clearTimeout(readyTimer);
      readyTimer = undefined;
    }
  };
  readyTimer = setTimeout(() => {
    onReadyTimeout?.();
    dispose();
  }, readyTimeoutMs);
  try {
    const subscribed = subscribe(waveform, { dispose, markReady });
    if (disposed) subscribed.forEach((unsubscribe) => unsubscribe());
    else unsubscribers = subscribed;
  } catch (error) {
    dispose();
    throw error;
  }
  return {
    dispose,
    waveform,
  };
}

export function seekRatioFromBounds(
  clientX: number,
  bounds: Pick<DOMRect, "left" | "width">,
) {
  if (bounds.width <= 0) return undefined;
  return Math.max(0, Math.min(1, (clientX - bounds.left) / bounds.width));
}

export function roundedMediaSecond(seconds: number | undefined) {
  return seconds !== undefined && Number.isFinite(seconds)
    ? Math.max(0, Math.floor(seconds))
    : 0;
}

function RecordingPlayer({
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
  const waveformRef = useRef<HTMLDivElement>(null);
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
  const [waveformMounted, setWaveformMounted] = useState(false);
  const [waveformRequested, setWaveformRequested] = useState(false);
  const recordingPath = item.playbackPath;
  const recordingSrc = useMemo(
    () => (isTauri() && recordingPath ? recordingPath : undefined),
    [recordingPath],
  );
  const durationSeconds = nativeMetadata && nativeMetadata.recordingSrc === recordingSrc
    ? nativeMetadata.durationSeconds
    : undefined;
  const waveformMode = durationSeconds === undefined
    ? "pending"
    : waveformMounted
      ? "decoded"
      : "lightweight";
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
    setWaveformMounted(false);
    setWaveformRequested(false);
  }, [recordingSrc]);

  useEffect(() => {
    const audio = audioRef.current;
    const container = waveformRef.current;
    const byteLength = item.playbackByteLength;
    if (
      !audio ||
      !container ||
      !recordingSrc ||
      durationSeconds === undefined ||
      byteLength === undefined
    ) return;

    const mounted = mountDecodedWaveform({
      byteLength,
      create: () => WaveSurfer.create({
        barGap: 2,
        barMinHeight: 3,
        barRadius: 999,
        barWidth: 3,
        container,
        cursorWidth: 0,
        height: 56,
        hideScrollbar: true,
        interact: false,
        media: audio,
        normalize: true,
        progressColor: "#034f46",
        sampleRate: decodedWaveformSampleRate,
        waveColor: "rgba(117, 111, 102, 0.28)",
      }),
      durationSeconds,
      onReadyTimeout: () => setWaveformMounted(false),
      requested: waveformRequested,
      subscribe: (waveSurfer, lifecycle) => [
        waveSurfer.on("ready", () => {
          lifecycle.markReady();
          setWaveformMounted(true);
          setDisplaySeconds(audio.currentTime);
        }),
        waveSurfer.on("error", () => {
          lifecycle.dispose();
          setPlaying(!audio.paused);
          setWaveformMounted(false);
        }),
      ],
    });
    if (!mounted) return;

    return () => {
      mounted.dispose();
      setWaveformMounted(false);
    };
  }, [durationSeconds, item.playbackByteLength, recordingSrc, waveformRequested]);

  if (!recordingPath || !recordingSrc) return null;

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
    setWaveformRequested(true);
    toggleNativePlayback();
  }

  function toggleNativePlayback() {
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
    <section className={cn("grid gap-3 border-b", variant === "modal" ? "bg-background px-8 py-6" : "bg-muted/40 p-4 sm:p-5")} aria-label="Recording playback">
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2">
          <FileAudio className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="truncate text-sm font-medium">{item.name}</span>
              <Badge variant="secondary">
                {durationSeconds === undefined ? "Local file" : formatElapsed(Math.floor(durationSeconds))}
              </Badge>
            </div>
            <p className="truncate text-xs text-muted-foreground" id={statusId} title={item.path}>
              {recordingStatus}
            </p>
          </div>
        </div>
        <ButtonGroup aria-label="Recording actions">
          <Button
            aria-label={`Open recording ${item.name}`}
            onClick={() => onOpen(item.path)}
            size="sm"
            type="button"
            variant="secondary"
          >
            <FileAudio data-icon="inline-start" />
            Open
          </Button>
          <Button
            aria-label={`Reveal recording ${item.name}`}
            onClick={() => onReveal(item.path)}
            size="sm"
            type="button"
            variant="ghost"
          >
            <FolderOpen data-icon="inline-start" />
            Reveal
          </Button>
        </ButtonGroup>
      </div>
      <div className={cn("rounded-lg border bg-background p-3", variant === "modal" && "rounded-2xl border-0 bg-muted/35 p-5 shadow-[0_0_0_1px_rgba(0,0,0,0.04)]")}>
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
            data-waveform-mounted={waveformMounted ? "true" : undefined}
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
            ref={waveformRef}
            role="slider"
            tabIndex={canSeek ? 0 : -1}
          >
            {waveformMode === "decoded" ? null : (
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
            )}
          </div>
        </div>
        <div className="mt-2 flex items-center justify-between gap-3 text-xs text-muted-foreground">
          <span>{playing ? "Playing" : recordingStatus}</span>
          <span className="tabular-nums">
            {formatElapsed(currentSeconds)}
            {durationSeconds === undefined ? null : ` / ${formatElapsed(Math.floor(durationSeconds))}`}
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

export function TranscriptPanel({
  className,
  elapsedSeconds,
  item,
  onCopy,
  onOpen,
  onOpenHelp,
  onRetry,
  onReveal,
  running,
  text,
  variant = "panel",
}: {
  className?: string;
  elapsedSeconds: number;
  item?: RecordingJobView;
  onCopy: (item: RecordingJobView) => void;
  onOpen: (path: string) => void;
  onOpenHelp?: () => void;
  onRetry: (id: number) => void;
  onReveal: (path: string) => void;
  running: boolean;
  text?: string;
  variant?: "panel" | "modal";
}) {
  const output = item?.output;
  const isDone = isRecordingFinished(item?.status);
  const isRunning = item ? isRecordingActive(item.status) : false;
  const isError = item?.status === "failed";
  const transcriptText = projectTranscriptText(text);

  useEffect(() => {
    if (!isDone || !item?.output) return;

    const copyItem = item;

    function onKeyDown(event: globalThis.KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === "c") {
        event.preventDefault();
        void onCopy(copyItem);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isDone, item, onCopy]);

  return (
    <Card
      className={cn(
        variant === "modal"
          ? "flex h-full min-h-0 min-w-0 flex-col gap-0 rounded-none bg-background py-0"
          : "surface-workspace-inset flex min-h-[420px] min-w-0 flex-col bg-card py-0 xl:sticky xl:top-5 xl:min-h-[calc(100vh-180px)]",
        className,
      )}
    >
      <CardHeader className={cn("gap-3 border-b", variant === "modal" ? "p-8 pb-6" : "p-4 sm:p-5")}>
        <div className="min-w-0">
          <CardTitle className="truncate text-lg">{item?.name ?? "Transcript"}</CardTitle>
          <CardDescription>
            {isDone
              ? (
                <>
                  Saved locally
                  <span className="hidden sm:inline">
                    {" "}
                    ·{" "}
                    <KbdGroup className="inline-flex align-middle">
                      <Kbd>Ctrl</Kbd>
                      <Kbd>Shift</Kbd>
                      <Kbd>C</Kbd>
                    </KbdGroup>{" "}
                    to copy
                  </span>
                </>
              )
              : isRunning
                ? item?.progressMessage ??
                  (item ? recordingActivityLabel(item.status) : "Working")
                : isError
                  ? "Transcription failed"
                  : item
                    ? "Waiting in queue"
                    : "Select a file or finish a transcription to preview text here."}
          </CardDescription>
        </div>
        {output ? (
          <CardAction className="col-span-full col-start-1 row-span-1 row-start-2 w-full justify-self-stretch sm:col-span-1 sm:col-start-2 sm:row-span-2 sm:row-start-1 sm:w-auto sm:justify-self-end">
            <ButtonGroup
              aria-label="Transcript actions"
              className="w-full sm:w-auto [&>[data-slot=button]]:flex-1 sm:[&>[data-slot=button]]:flex-none"
            >
              <Button
                aria-label={`Copy transcript for ${item.name}`}
                onClick={() => void onCopy(item)}
                size="sm"
                type="button"
              >
                <Copy data-icon="inline-start" />
                Copy
              </Button>
              <Button
                aria-label={`Open transcript for ${item.name}`}
                onClick={() => onOpen(output)}
                size="sm"
                type="button"
                variant="secondary"
              >
                <FileText data-icon="inline-start" />
                Open
              </Button>
              <Button
                aria-label={`Reveal transcript for ${item.name}`}
                onClick={() => onReveal(output)}
                size="sm"
                type="button"
                variant="ghost"
              >
                <FolderOpen data-icon="inline-start" />
                Reveal
              </Button>
            </ButtonGroup>
          </CardAction>
        ) : null}
      </CardHeader>
      <CardContent className="flex min-h-0 flex-1 flex-col p-0">
        {item ? <RecordingPlayer item={item} onOpen={onOpen} onReveal={onReveal} variant={variant} /> : null}
        <ScrollArea className="min-h-[280px] flex-1 bg-[var(--surface-transcript)]">
          <div className={cn("min-h-[280px]", variant === "modal" ? "p-8 pt-7" : "p-5")}>
            {item?.status === "partial" && item.error ? (
              <Alert className="mb-5">
                <HelpCircle />
                <AlertDescription>{item.error}</AlertDescription>
              </Alert>
            ) : null}
            {isDone ? (
              transcriptText.state === "ready" ? (
                <pre className="whitespace-pre-wrap break-words text-[15px] leading-7 text-foreground">{transcriptText.text}</pre>
              ) : transcriptText.state === "empty" ? (
                <p className="text-[15px] leading-7 text-muted-foreground">{transcriptText.text}</p>
              ) : (
                <div className="flex flex-col gap-3">
                  <Skeleton className="h-4 w-3/4" />
                  <Skeleton className="h-4 w-full" />
                  <Skeleton className="h-4 w-5/6" />
                  <p className="text-sm text-muted-foreground">{transcriptText.text}</p>
                </div>
              )
            ) : isError ? (
              <div className="flex flex-col gap-4">
                <Alert variant="destructive">
                  <HelpCircle />
                  <AlertDescription>{item.error}</AlertDescription>
                </Alert>
                <Button className="w-fit" onClick={() => onRetry(item.id)} type="button">
                  <RotateCcw data-icon="inline-start" />
                  Retry transcription
                </Button>
              </div>
            ) : item ? (
              <div className="flex flex-col gap-3">
                <Badge variant="secondary">
                  {isRunning ? (
                    item.progressMessage ? (
                      <>
                        {item.progressMessage}
                        {item.progressPercent !== undefined ? (
                          <>
                            {" "}
                            · <span className="tabular-nums">{item.progressPercent}%</span>
                          </>
                        ) : null}
                      </>
                    ) : elapsedSeconds ? (
                      <>
                        Transcribing · <span className="tabular-nums">{formatElapsed(elapsedSeconds)}</span>
                      </>
                    ) : (
                      "Transcribing"
                    )
                  ) : running ? (
                    "Transcribing"
                  ) : (
                    "Queued"
                  )}
                </Badge>
                <p className="text-[15px] leading-7 text-muted-foreground">
                  {item.route === "serverBatch"
                    ? queuedServerMessage
                    : "The finished transcript will appear here as soon as the local run completes."}
                </p>
              </div>
            ) : (
              <Empty className="border-0 bg-transparent">
                <EmptyMedia>
                  <FileText />
                </EmptyMedia>
                <div>
                  <EmptyTitle>No transcript selected</EmptyTitle>
                  <EmptyDescription>
                    Drop a recording on Transcribe or pick one from Home.
                  </EmptyDescription>
                  {onOpenHelp ? (
                    <Button
                      className="mt-2 h-auto px-0 text-muted-foreground"
                      onClick={onOpenHelp}
                      size="sm"
                      type="button"
                      variant="link"
                    >
                      How this works
                    </Button>
                  ) : null}
                </div>
              </Empty>
            )}
          </div>
        </ScrollArea>
      </CardContent>
    </Card>
  );
}
