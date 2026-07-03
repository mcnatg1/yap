import { useEffect, useId, useMemo, useRef, useState } from "react";
import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { Copy, FileAudio, FileText, FolderOpen, HelpCircle, Pause, Play, RotateCcw } from "lucide-react";

import { type UploadItem } from "@/components/stacked-upload";
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
import { formatElapsed } from "@/lib/app-types";

const WAVEFORM_BAR_COUNT = 64;
const FALLBACK_WAVEFORM_BARS = [
  34, 58, 45, 72, 40, 66, 82, 52, 38, 74, 61, 44, 86, 56, 70, 42,
  64, 88, 50, 36, 76, 59, 46, 80, 54, 68, 39, 73, 62, 48, 84, 57,
  43, 71, 90, 53, 37, 78, 60, 45, 83, 55, 69, 41, 75, 63, 49, 87,
  35, 65, 47, 79, 58, 42, 74, 61, 52, 86, 44, 70, 55, 39, 76, 63,
];

export function waveformPeaks(audioBuffer: AudioBuffer, count = WAVEFORM_BAR_COUNT) {
  const channels = Array.from({ length: audioBuffer.numberOfChannels }, (_, index) =>
    audioBuffer.getChannelData(index),
  );
  const samplesPerBar = Math.max(1, Math.floor(audioBuffer.length / count));
  const peaks: number[] = [];
  let loudest = 0;

  for (let bar = 0; bar < count; bar += 1) {
    const start = bar * samplesPerBar;
    const end = Math.min(audioBuffer.length, start + samplesPerBar);
    const stride = Math.max(1, Math.floor((end - start) / 512));
    let sum = 0;
    let seen = 0;

    for (let sampleIndex = start; sampleIndex < end; sampleIndex += stride) {
      const mixed = channels.reduce((total, channel) => total + Math.abs(channel[sampleIndex] ?? 0), 0);
      const sample = mixed / channels.length;
      sum += sample * sample;
      seen += 1;
    }

    const peak = Math.sqrt(sum / Math.max(1, seen));
    loudest = Math.max(loudest, peak);
    peaks.push(peak);
  }

  if (!loudest) return Array.from({ length: count }, () => 16);
  return peaks.map((peak) => Math.round(16 + (peak / loudest) * 84));
}

function RecordingPlayer({
  item,
  onOpen,
  onReveal,
}: {
  item: UploadItem;
  onOpen: (path: string) => void;
  onReveal: (path: string) => void;
}) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const draggingRef = useRef(false);
  const displayedSecondRef = useRef(0);
  const waveformFillRef = useRef<HTMLDivElement>(null);
  const waveformRef = useRef<HTMLDivElement>(null);
  const waveformBoundsRef = useRef<DOMRect | undefined>(undefined);
  const pendingSeekClientXRef = useRef<number | undefined>(undefined);
  const seekFrameRef = useRef<number | undefined>(undefined);
  const statusId = useId();
  const errorId = useId();
  const [currentSeconds, setCurrentSeconds] = useState(0);
  const [durationSeconds, setDurationSeconds] = useState<number>();
  const [failed, setFailed] = useState(false);
  const [peaks, setPeaks] = useState(FALLBACK_WAVEFORM_BARS);
  const [playing, setPlaying] = useState(false);
  const recordingSrc = useMemo(() => (isTauri() ? convertFileSrc(item.path) : undefined), [item.path]);
  const recordingStatus = failed
    ? "Playback unavailable"
    : item.status === "done"
      ? "Transcript saved"
      : item.status === "running"
        ? "Transcribing locally"
        : item.status === "error"
          ? "Transcription failed"
          : "Queued";
  const canSeek = !failed && durationSeconds !== undefined && durationSeconds > 0;

  useEffect(() => {
    setCurrentSeconds(0);
    displayedSecondRef.current = 0;
    setFailed(false);
    setPlaying(false);
    setDurationSeconds(undefined);
    paintProgress(0, 1);
  }, [recordingSrc]);

  useEffect(() => {
    if (!recordingSrc) return;

    const src = recordingSrc;
    let cancelled = false;
    let audioContext: AudioContext | undefined;
    setPeaks(FALLBACK_WAVEFORM_BARS);

    async function loadWaveform() {
      try {
        const AudioContextCtor =
          window.AudioContext ??
          (window as Window & typeof globalThis & { webkitAudioContext?: typeof AudioContext })
            .webkitAudioContext;

        if (!AudioContextCtor) return;

        const response = await fetch(src);
        if (!response.ok) throw new Error("Recording unavailable");
        const data = await response.arrayBuffer();
        audioContext = new AudioContextCtor();
        const audioBuffer = await audioContext.decodeAudioData(data);
        if (!cancelled) setPeaks(waveformPeaks(audioBuffer));
      } catch {
        if (!cancelled) setPeaks(FALLBACK_WAVEFORM_BARS);
      } finally {
        if (audioContext?.state !== "closed") void audioContext?.close().catch(() => undefined);
      }
    }

    // ponytail: selected-file decode in the UI; move to a worker if long recordings stutter.
    void loadWaveform();
    return () => {
      cancelled = true;
      if (audioContext?.state !== "closed") void audioContext?.close().catch(() => undefined);
    };
  }, [recordingSrc]);

  useEffect(() => {
    if (!playing) return;

    let frame = 0;
    function tick() {
      const audio = audioRef.current;
      if (audio) {
        paintProgress(audio.currentTime, audio.duration);
        setDisplaySeconds(audio.currentTime);
      }
      frame = window.requestAnimationFrame(tick);
    }

    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, [playing]);

  useEffect(() => {
    return () => {
      if (seekFrameRef.current) window.cancelAnimationFrame(seekFrameRef.current);
    };
  }, []);

  if (!recordingSrc) return null;

  function paintProgress(seconds: number, duration: number | undefined = durationSeconds) {
    const resolvedDuration = duration && Number.isFinite(duration) ? duration : 0;
    const ratio = resolvedDuration ? Math.max(0, Math.min(1, seconds / resolvedDuration)) : 0;
    if (waveformFillRef.current) {
      waveformFillRef.current.style.clipPath = `inset(0 ${100 - ratio * 100}% 0 0)`;
    }
  }

  function setDisplaySeconds(seconds: number) {
    const wholeSeconds = Math.floor(seconds);
    if (displayedSecondRef.current === wholeSeconds) return;
    displayedSecondRef.current = wholeSeconds;
    setCurrentSeconds(wholeSeconds);
  }

  function seekToRatio(ratio: number) {
    const audio = audioRef.current;
    const duration = durationSeconds ?? audio?.duration;
    if (!audio || !duration || !Number.isFinite(duration)) return;

    const nextSeconds = Math.max(0, Math.min(duration, ratio * duration));
    audio.currentTime = nextSeconds;
    paintProgress(nextSeconds, duration);
    setDisplaySeconds(nextSeconds);
  }

  function seekBy(deltaSeconds: number) {
    const audio = audioRef.current;
    const duration = durationSeconds ?? audio?.duration;
    if (!duration || !Number.isFinite(duration)) return;
    seekToRatio(((audio?.currentTime ?? currentSeconds) + deltaSeconds) / duration);
  }

  function seekFromClientX(clientX: number) {
    const bounds = waveformBoundsRef.current ?? waveformRef.current?.getBoundingClientRect();
    if (!bounds?.width) return;
    seekToRatio((clientX - bounds.left) / bounds.width);
  }

  function scheduleSeekFromClientX(clientX: number) {
    pendingSeekClientXRef.current = clientX;
    if (seekFrameRef.current) return;

    seekFrameRef.current = window.requestAnimationFrame(() => {
      seekFrameRef.current = undefined;
      if (pendingSeekClientXRef.current === undefined) return;
      seekFromClientX(pendingSeekClientXRef.current);
    });
  }

  function endDrag(clientX?: number) {
    draggingRef.current = false;
    if (seekFrameRef.current) {
      window.cancelAnimationFrame(seekFrameRef.current);
      seekFrameRef.current = undefined;
    }
    if (clientX !== undefined) seekFromClientX(clientX);
    pendingSeekClientXRef.current = undefined;
    waveformBoundsRef.current = undefined;
  }

  function togglePlayback() {
    const audio = audioRef.current;
    if (!audio) return;
    if (audio.paused) {
      void audio.play().catch(() => setFailed(true));
      return;
    }
    audio.pause();
  }

  return (
    <section className="grid gap-3 border-b bg-muted/40 p-4 sm:p-5" aria-label="Recording playback">
      <div className="flex min-w-0 flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
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
        <ButtonGroup
          aria-label="Recording actions"
          className="w-full [&>[data-slot=button]]:flex-1 sm:w-auto sm:[&>[data-slot=button]]:flex-none"
        >
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
      <div className="rounded-lg border bg-background p-3">
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
            aria-valuemax={Math.round(durationSeconds ?? 0)}
            aria-valuemin={0}
            aria-valuenow={currentSeconds}
            aria-valuetext={`${formatElapsed(currentSeconds)}${durationSeconds === undefined ? "" : ` of ${formatElapsed(Math.floor(durationSeconds))}`}`}
            className="relative h-14 min-w-0 flex-1 cursor-pointer touch-none overflow-hidden rounded-md bg-muted/60 px-3 outline-none ring-offset-background transition-[background-color,box-shadow] duration-150 ease-out hover:bg-muted/80 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 aria-disabled:cursor-default aria-disabled:opacity-70 aria-disabled:hover:bg-muted/60"
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
            onPointerDown={(event) => {
              if (!canSeek) return;
              event.preventDefault();
              draggingRef.current = true;
              waveformBoundsRef.current = event.currentTarget.getBoundingClientRect();
              event.currentTarget.setPointerCapture(event.pointerId);
              seekFromClientX(event.clientX);
            }}
            onPointerMove={(event) => {
              if (!draggingRef.current) return;
              event.preventDefault();
              scheduleSeekFromClientX(event.clientX);
            }}
            onPointerUp={(event) => {
              endDrag(event.clientX);
              if (event.currentTarget.hasPointerCapture(event.pointerId)) {
                event.currentTarget.releasePointerCapture(event.pointerId);
              }
            }}
            onPointerCancel={() => {
              endDrag();
            }}
            ref={waveformRef}
            role="slider"
            tabIndex={canSeek ? 0 : -1}
          >
            <div className="absolute inset-x-3 inset-y-0 flex items-center gap-1" aria-hidden="true">
              {peaks.map((height, index) => (
                <span
                  className="w-1 flex-1 rounded-full bg-muted-foreground/25"
                  key={`${height}-${index}`}
                  style={{ height: `${height}%` }}
                />
              ))}
            </div>
            <div
              className="absolute inset-x-3 inset-y-0 flex items-center gap-1"
              ref={waveformFillRef}
              style={{ clipPath: "inset(0 100% 0 0)", willChange: "clip-path" }}
              aria-hidden="true"
            >
              {peaks.map((height, index) => (
                <span
                  className="w-1 flex-1 rounded-full bg-primary/75"
                  key={`${height}-${index}`}
                  style={{ height: `${height}%` }}
                />
              ))}
            </div>
          </div>
        </div>
        <div className="mt-2 flex items-center justify-between gap-3 text-xs text-muted-foreground">
          <span>{playing ? "Playing" : recordingStatus}</span>
          <span className="tabular-nums">
            {formatElapsed(currentSeconds)}
            {durationSeconds === undefined ? null : ` / ${formatElapsed(Math.floor(durationSeconds))}`}
          </span>
        </div>
        <audio
          aria-describedby={failed ? `${statusId} ${errorId}` : statusId}
          aria-label={`Play recording ${item.name}`}
          aria-hidden="true"
          className="sr-only"
          key={recordingSrc}
          onEnded={(event) => {
            const seconds = event.currentTarget.duration;
            setPlaying(false);
            if (!Number.isFinite(seconds)) return;
            paintProgress(seconds, seconds);
            setDisplaySeconds(seconds);
          }}
          onError={() => {
            setFailed(true);
            setPlaying(false);
          }}
          onLoadedMetadata={(event) => {
            const seconds = event.currentTarget.duration;
            setDurationSeconds(Number.isFinite(seconds) ? seconds : undefined);
            paintProgress(event.currentTarget.currentTime, seconds);
            setDisplaySeconds(event.currentTarget.currentTime);
          }}
          onPause={() => setPlaying(false)}
          onPlay={() => setPlaying(true)}
          onTimeUpdate={(event) => {
            if (playing) return;
            paintProgress(event.currentTarget.currentTime, event.currentTarget.duration);
            setDisplaySeconds(event.currentTarget.currentTime);
          }}
          preload="metadata"
          ref={audioRef}
          src={recordingSrc}
        />
      </div>
      {failed ? (
        <p className="text-sm text-muted-foreground" id={errorId}>
          This recording is unsupported, moved, or unavailable in the app. Open it from disk instead.
        </p>
      ) : null}
    </section>
  );
}

export function TranscriptPanel({
  elapsedSeconds,
  item,
  onCopy,
  onOpen,
  onOpenHelp,
  onRetry,
  onReveal,
  running,
  text,
}: {
  elapsedSeconds: number;
  item?: UploadItem;
  onCopy: (item: UploadItem) => void;
  onOpen: (path: string) => void;
  onOpenHelp?: () => void;
  onRetry: (id: number) => void;
  onReveal: (path: string) => void;
  running: boolean;
  text?: string;
}) {
  const output = item?.output;
  const isDone = item?.status === "done";
  const isRunning = item?.status === "running";
  const isError = item?.status === "error";

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
    <Card className="surface-workspace-inset flex min-h-[420px] min-w-0 flex-col bg-card py-0 xl:sticky xl:top-5 xl:min-h-[calc(100vh-180px)]">
      <CardHeader className="gap-3 border-b p-4 sm:p-5">
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
                  (elapsedSeconds
                    ? `Transcribing locally · ${formatElapsed(elapsedSeconds)}`
                    : "Transcribing locally…")
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
        {item ? <RecordingPlayer item={item} onOpen={onOpen} onReveal={onReveal} /> : null}
        <ScrollArea className="min-h-[280px] flex-1 bg-[var(--surface-transcript)]">
          <div className="min-h-[280px] p-5">
            {isDone ? (
              text ? (
                <pre className="whitespace-pre-wrap break-words text-[15px] leading-7 text-foreground">{text}</pre>
              ) : (
                <div className="flex flex-col gap-3">
                  <Skeleton className="h-4 w-3/4" />
                  <Skeleton className="h-4 w-full" />
                  <Skeleton className="h-4 w-5/6" />
                  <p className="text-sm text-muted-foreground">Loading transcript…</p>
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
                  The finished transcript will appear here as soon as the local run completes.
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
                    Drop a recording on Transcribe or pick one from Transcripts.
                  </EmptyDescription>
                  <p className="mt-2 text-sm text-muted-foreground">
                    <KbdGroup className="inline-flex align-middle">
                      <Kbd>Ctrl</Kbd>
                      <Kbd>K</Kbd>
                    </KbdGroup>{" "}
                    opens search and quick actions.
                  </p>
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
