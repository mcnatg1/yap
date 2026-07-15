import { Copy } from "@phosphor-icons/react/Copy";
import { FileText } from "@phosphor-icons/react/FileText";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { Question as HelpCircle } from "@phosphor-icons/react/Question";
import { ArrowCounterClockwise as RotateCcw } from "@phosphor-icons/react/ArrowCounterClockwise";
import { useEffect } from "react";

import { RecordingPlayer } from "@/components/playback/recording-player";
import { recordingActivityLabel } from "@/components/playback/recording-status";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ButtonGroup } from "@/components/ui/button-group";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import {
  formatElapsed,
  isRecordingActive,
  isRecordingFinished,
  queuedServerMessage,
  type RecordingJobView,
} from "@/lib/app-types";
import { projectTranscriptText } from "@/lib/transcript-text";
import { cn } from "@/lib/utils";

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
  onRetry: (id: string) => void;
  onReveal: (path: string) => void;
  running: boolean;
  text?: string;
  variant?: "panel" | "modal";
}) {
  const output = item?.outputPath;
  const isDone = isRecordingFinished(item?.status);
  const isRunning = item ? isRecordingActive(item.status) : false;
  const isError = item?.status === "failed";
  const transcriptText = projectTranscriptText(text);

  useEffect(() => {
    if (!isDone || !item?.outputPath) return;

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
                ? item ? recordingActivityLabel(item.status) : "Working"
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
        {item ? (
          <RecordingPlayer item={item} onOpen={onOpen} onReveal={onReveal} variant={variant} />
        ) : null}
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
                <pre className="whitespace-pre-wrap break-words text-[15px] leading-7 text-foreground">
                  {transcriptText.text}
                </pre>
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
                    elapsedSeconds ? (
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
