import { Sparkles, Trash2 } from "lucide-react";

import { StackedUpload, type UploadItem } from "@/components/stacked-upload";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
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
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Progress } from "@/components/ui/progress";
import { Spinner } from "@/components/ui/spinner";
import { formatElapsed } from "@/lib/app-types";

export function QueuePanel({
  completed,
  elapsedSeconds,
  hasRunnable,
  onClear,
  onRemove,
  onRetry,
  onReveal,
  onRun,
  onSelect,
  queue,
  queueProgress,
  running,
  runningItem,
  selectedId,
}: {
  completed: number;
  elapsedSeconds: number;
  hasRunnable: boolean;
  onClear: () => void;
  onRemove: (id: number) => void;
  onRetry: (id: number) => void;
  onReveal: (path: string) => void;
  onRun: () => void;
  onSelect: (id: number) => void;
  queue: UploadItem[];
  queueProgress: number;
  running: boolean;
  runningItem?: UploadItem;
  selectedId?: number;
}) {
  const TranscribeIcon = running ? Spinner : Sparkles;

  return (
    <Card className="surface-workspace-inset h-full min-w-0 bg-card py-0">
      <CardHeader className="p-4 sm:p-5">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-xl">
            Queue
            <Badge className="tabular-nums" variant="secondary">
              {queue.length}
            </Badge>
          </CardTitle>
          <CardDescription>
            {running && runningItem ? (
              <>
                <span className="font-medium text-foreground">{runningItem.name}</span>
                {" · "}
                {runningItem.progressMessage ?? "Transcribing"}
                {elapsedSeconds ? (
                  <>
                    {" "}
                    · <span className="tabular-nums">{formatElapsed(elapsedSeconds)}</span>
                  </>
                ) : null}
                {runningItem.progressPercent !== undefined ? (
                  <>
                    {" "}
                    · <span className="tabular-nums">{runningItem.progressPercent}%</span>
                  </>
                ) : null}
              </>
            ) : completed ? (
              <>
                <span className="tabular-nums">{completed}</span> transcript{completed === 1 ? "" : "s"} ready
              </>
            ) : (
              "Drop files on Transcribe and run them in place"
            )}
          </CardDescription>
        </div>
        <CardAction className="col-span-full col-start-1 row-span-1 row-start-2 w-full justify-self-stretch sm:col-span-1 sm:col-start-2 sm:row-span-2 sm:row-start-1 sm:w-auto sm:justify-self-end">
          <ButtonGroup
            aria-label="Queue actions"
            className="w-full sm:w-auto [&>[data-slot=button]]:flex-1 sm:[&>[data-slot=button]]:flex-none"
          >
            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button disabled={running || !queue.length} size="sm" type="button" variant="outline">
                  <Trash2 data-icon="inline-start" />
                  Clear
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>Clear the queue?</AlertDialogTitle>
                  <AlertDialogDescription>
                    This removes the queued files from Yap. Saved transcript files and history stay untouched.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Cancel</AlertDialogCancel>
                  <AlertDialogAction
                    className="bg-destructive text-white hover:bg-destructive/90 focus-visible:ring-destructive/20"
                    onClick={onClear}
                  >
                    Clear queue
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
            <Button disabled={running || !hasRunnable} onClick={onRun} size="sm" type="button">
              <span className="relative inline-flex size-4 shrink-0 items-center justify-center" data-icon="inline-start">
                <TranscribeIcon />
              </span>
              Transcribe
            </Button>
          </ButtonGroup>
        </CardAction>
      </CardHeader>
      <CardContent className="p-4 sm:p-5">
        {queue.length ? (
          <Field className="mb-4 gap-2">
            <div className="flex items-center justify-between gap-3">
              <FieldLabel>Queue progress</FieldLabel>
              <FieldDescription className="tabular-nums">
                {completed} of {queue.length}
              </FieldDescription>
            </div>
            <Progress value={queueProgress} />
          </Field>
        ) : null}
        <StackedUpload
          elapsedSeconds={elapsedSeconds}
          items={queue}
          onRemove={onRemove}
          onRetry={onRetry}
          onReveal={onReveal}
          onSelect={onSelect}
          selectedId={selectedId}
        />
      </CardContent>
    </Card>
  );
}
