import { Trash as Trash2 } from "@phosphor-icons/react/Trash";
import { WarningCircle } from "@phosphor-icons/react/WarningCircle";

import { StackedUpload } from "@/components/stacked-upload";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { RecordingJobView } from "@/lib/recording-job";

export function QueuePanel({
  legacyDiscardAllowed,
  onClear,
  onDiscardLegacyQueue,
  onRemove,
  onReveal,
  onRetryMigration,
  onSelect,
  queue,
  migrationError,
  migrationPending,
  selectedId,
}: {
  legacyDiscardAllowed: boolean;
  onClear: () => void | Promise<void>;
  onDiscardLegacyQueue: () => void;
  onRemove: (id: string) => void | Promise<void>;
  onReveal: (path: string) => void;
  onRetryMigration: () => void | Promise<unknown>;
  onSelect: (id: string) => void;
  queue: RecordingJobView[];
  migrationError?: string;
  migrationPending: boolean;
  selectedId?: string;
}) {
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
            {queue.length ? (
              <>
                <span className="tabular-nums">{queue.length}</span> recording{queue.length === 1 ? "" : "s"} waiting for the organization server.
              </>
            ) : (
              "Choose files above to add them to the organization server queue."
            )}
          </CardDescription>
        </div>
        <CardAction className="col-span-full col-start-1 row-span-1 row-start-2 w-full justify-self-stretch sm:col-span-1 sm:col-start-2 sm:row-span-2 sm:row-start-1 sm:w-auto sm:justify-self-end">
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button className="w-full sm:w-auto" disabled={!queue.length} size="sm" type="button" variant="outline">
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
        </CardAction>
      </CardHeader>
      <CardContent className="p-4 sm:p-5">
        {migrationError ? (
          <Alert className="mb-4" variant="destructive">
            <WarningCircle />
            <AlertDescription className="flex flex-wrap items-center justify-between gap-3">
              <span>{migrationError}</span>
              <div className="flex flex-wrap items-center gap-2">
                <Button onClick={() => void onRetryMigration()} size="sm" type="button" variant="outline">
                  Retry restore
                </Button>
                {legacyDiscardAllowed ? (
                  <AlertDialog>
                    <AlertDialogTrigger asChild>
                      <Button size="sm" type="button" variant="destructive">
                        Discard old queue
                      </Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent>
                      <AlertDialogHeader>
                        <AlertDialogTitle>Discard the old queue?</AlertDialogTitle>
                        <AlertDialogDescription>
                          This permanently removes the legacy queue data Yap could not restore. Audio files and current native recording jobs stay untouched. This cannot be undone.
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel>Keep old queue</AlertDialogCancel>
                        <AlertDialogAction
                          className="bg-destructive text-white hover:bg-destructive/90 focus-visible:ring-destructive/20"
                          onClick={onDiscardLegacyQueue}
                        >
                          Discard old queue
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                ) : null}
              </div>
            </AlertDescription>
          </Alert>
        ) : migrationPending ? (
          <p className="mb-4 text-sm text-muted-foreground">Restoring queued recordings…</p>
        ) : null}
        <StackedUpload
          items={queue}
          onRemove={onRemove}
          onReveal={onReveal}
          onSelect={onSelect}
          selectedId={selectedId}
        />
      </CardContent>
    </Card>
  );
}
