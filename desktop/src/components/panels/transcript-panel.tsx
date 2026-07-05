import { useEffect, useId, useMemo, useRef, useState } from "react";
import { convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { Copy, FileAudio, FileText, FolderOpen, HelpCircle, Pause, Play, RotateCcw } from "lucide-react";
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
  type RecordingJobStatus,
  type RecordingJobView,
} from "@/lib/app-types";
import { cn } from "@/lib/utils";

function recordingActivityLabel(status: RecordingJobStatus, elapsedSeconds?: number) {
  switch (status) {
    case "uploading":
      return "Uploading";
    case "server_processing_cohere":
      return "Processing on server";
    case "diarization_running":
      return "Finding speakers";
    case "saving":
      return "Saving";
    case "local_transcribing":
      return elapsedSeconds ? `Transcribing locally · ${formatElapsed(elapsedSeconds)}` : "Transcribing locally...";
    default:
      return "Working";
  }
}

function RecordingPlayer({
  item,
  onOpen,
  onReveal,
}: {
  item: RecordingJobView;
  onOpen: (path: string) => void;
  onReveal: (path: string) => void;
}) {
  const displayedSecondRef = useRef(0);
  const waveformRef = useRef<HTMLDivElement>(null);
  const waveSurferRef = useRef<WaveSurfer | undefined>(undefined);
  const statusId = useId();
  const errorId = useId();
  const [currentSeconds, setCurrentSeconds] = useState(0);
  const [durationSeconds, setDurationSeconds] = useState<number>();
  const [failed, setFailed] = useState(false);
  const [playing, setPlaying] = useState(false);
  const recordingSrc = useMemo(() => (isTauri() ? convertFileSrc(item.path) : undefined), [item.path]);
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
    displayedSecondRef.current = 0;
    setFailed(false);
    setPlaying(false);
    setDurationSeconds(undefined);
  }, [recordingSrc]);

  useEffect(() => {
    const container = waveformRef.current;
    if (!container || !recordingSrc) return;

    const waveSurfer = WaveSurfer.create({
      barGap: 2,
      barMinHeight: 3,
      barRadius: 999,
      barWidth: 3,
      container,
      cursorWidth: 0,
      dragToSeek: true,
      height: 56,
      hideScrollbar: true,
      normalize: true,
      progressColor: "#034f46",
      url: recordingSrc,
      waveColor: "rgba(117, 111, 102, 0.28)",
    });
    waveSurferRef.current = waveSurfer;

    const unsubscribers = [
      waveSurfer.on("ready", (duration) => {
        setDurationSeconds(Number.isFinite(duration) ? duration : undefined);
        setFailed(false);
        setDisplaySeconds(waveSurfer.getCurrentTime());
      }),
      waveSurfer.on("timeupdate", setDisplaySeconds),
      waveSurfer.on("seeking", setDisplaySeconds),
      waveSurfer.on("interaction", setDisplaySeconds),
      waveSurfer.on("play", () => setPlaying(true)),
      waveSurfer.on("pause", () => setPlaying(false)),
      waveSurfer.on("finish", () => {
        setPlaying(false);
        setDisplaySeconds(waveSurfer.getDuration());
      }),
      waveSurfer.on("error", () => {
        setFailed(true);
        setPlaying(false);
      }),
    ];

    return () => {
      unsubscribers.forEach((unsubscribe) => unsubscribe());
      waveSurfer.destroy();
      if (waveSurferRef.current === waveSurfer) waveSurferRef.current = undefined;
    };
  }, [recordingSrc]);

  if (!recordingSrc) return null;

  function setDisplaySeconds(seconds: number) {
    const wholeSeconds = Math.floor(seconds);
    if (displayedSecondRef.current === wholeSeconds) return;
    displayedSecondRef.current = wholeSeconds;
    setCurrentSeconds(wholeSeconds);
  }

  function seekToRatio(ratio: number) {
    const waveSurfer = waveSurferRef.current;
    const duration = durationSeconds ?? waveSurfer?.getDuration();
    if (!waveSurfer || !duration || !Number.isFinite(duration)) return;

    const nextSeconds = Math.max(0, Math.min(duration, ratio * duration));
    waveSurfer.setTime(nextSeconds);
    setDisplaySeconds(nextSeconds);
  }

  function seekBy(deltaSeconds: number) {
    const waveSurfer = waveSurferRef.current;
    const duration = durationSeconds ?? waveSurfer?.getDuration();
    if (!waveSurfer || !duration || !Number.isFinite(duration)) return;
    seekToRatio((waveSurfer.getCurrentTime() + deltaSeconds) / duration);
  }

  function togglePlayback() {
    void waveSurferRef.current?.playPause().catch(() => setFailed(true));
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
            className="relative h-14 min-w-0 flex-1 cursor-pointer overflow-hidden rounded-md bg-muted/60 outline-none ring-offset-background transition-[background-color,box-shadow] duration-150 ease-out hover:bg-muted/80 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 aria-disabled:cursor-default aria-disabled:opacity-70 aria-disabled:hover:bg-muted/60"
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
            ref={waveformRef}
            role="slider"
            tabIndex={canSeek ? 0 : -1}
          />
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
}) {
  const output = item?.output;
  const isDone = isRecordingFinished(item?.status);
  const isRunning = item ? isRecordingActive(item.status) : false;
  const isError = item?.status === "failed";

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
        "surface-workspace-inset flex min-h-[420px] min-w-0 flex-col bg-card py-0 xl:sticky xl:top-5 xl:min-h-[calc(100vh-180px)]",
        className,
      )}
    >
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
                  (item ? recordingActivityLabel(item.status, elapsedSeconds) : "Working")
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
