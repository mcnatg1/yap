import { useMemo } from "react";
import { ArrowRight, FileAudio2, FileText } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { TranscriptHistoryEntry } from "@/history";
import { formatHistoryTime, groupHistoryByDay } from "@/lib/app-types";
import { cn } from "@/lib/utils";

function buildGlanceSummary(historyCount: number, queueCount: number, running: boolean) {
  const parts = [`${historyCount} saved`];

  if (queueCount) {
    parts.push(`${queueCount} in queue`);
  }

  parts.push(running ? "Transcribing" : queueCount ? "Queued" : "Ready");
  return parts.join(" · ");
}

export function HomePanel({
  history,
  onOpenTranscribe,
  onPickFiles,
  onSelectEntry,
  onViewAll,
  previewSnippet,
  queueCount,
  running,
}: {
  history: TranscriptHistoryEntry[];
  onOpenTranscribe: () => void;
  onPickFiles: () => void;
  onSelectEntry: (entry: TranscriptHistoryEntry) => void;
  onViewAll: () => void;
  previewSnippet?: (entry: TranscriptHistoryEntry) => string | undefined;
  queueCount: number;
  running: boolean;
}) {
  const todayEntries = useMemo(() => {
    const today = groupHistoryByDay(history).find((group) => group.label === "Today");
    return today?.entries ?? [];
  }, [history]);

  const glanceSummary = buildGlanceSummary(history.length, queueCount, running);

  return (
    <div className="mt-7 grid w-full min-w-0 gap-5 xl:grid-cols-[minmax(0,1fr)_280px]">
      <div className="min-w-0 space-y-5">
        {running ? (
          <button
            className={cn(
              "flex w-full items-center justify-between gap-3 rounded-lg border border-primary/20 bg-[var(--primary-soft)]/50 px-4 py-3 text-left transition-colors",
              "hover:bg-[var(--primary-soft)]/70 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
            )}
            onClick={onOpenTranscribe}
            type="button"
          >
            <span className="text-sm font-medium">Transcribing in progress</span>
            <span className="inline-flex shrink-0 items-center gap-1 text-sm text-primary">
              Continue
              <ArrowRight data-icon="inline-end" />
            </span>
          </button>
        ) : null}

        <Card className="surface-workspace-inset border-dashed bg-[var(--surface-transcript)] py-0">
          <CardContent className="flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0">
              <p className="text-lg font-semibold tracking-tight">Transcribe recordings</p>
              <p className="mt-1 text-sm leading-6 text-muted-foreground">
                Drop files on the transcribe page or{" "}
                <Button
                  className="h-auto p-0 text-sm font-normal"
                  onClick={onPickFiles}
                  type="button"
                  variant="link"
                >
                  choose files
                </Button>{" "}
                from your machine.
              </p>
            </div>
            <div className="shrink-0">
              <Button onClick={onOpenTranscribe} type="button">
                <FileAudio2 data-icon="inline-start" />
                Open transcribe
              </Button>
            </div>
          </CardContent>
        </Card>

        <section>
          <div className="mb-3 flex items-center justify-between gap-3">
            <h2 className="text-sm font-medium text-muted-foreground">Today</h2>
            {history.length ? (
              <Button className="h-auto px-2 py-1 text-xs" onClick={onViewAll} type="button" variant="ghost">
                View all
                <ArrowRight data-icon="inline-end" />
              </Button>
            ) : null}
          </div>

          {todayEntries.length ? (
            <ScrollArea className="h-[min(520px,calc(100vh-360px))] pr-3">
              <ul className="flex flex-col gap-1">
                {todayEntries.map((entry) => {
                  const snippet = previewSnippet?.(entry)?.trim();
                  const preview = snippet ? snippet.replace(/\s+/g, " ").slice(0, 220) : entry.name;

                  return (
                    <li key={entry.outputPath}>
                      <button
                        className={cn(
                          "flex w-full gap-4 rounded-lg border border-transparent px-3 py-3 text-left transition-colors",
                          "hover:border-border hover:bg-secondary/60 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50",
                        )}
                        onClick={() => onSelectEntry(entry)}
                        type="button"
                      >
                        <span className="w-16 shrink-0 pt-0.5 text-xs tabular-nums text-muted-foreground">
                          {formatHistoryTime(entry.createdAt)}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="line-clamp-3 text-sm leading-6 text-foreground">{preview}</span>
                          {snippet ? (
                            <span className="mt-1 block truncate text-xs text-muted-foreground">{entry.name}</span>
                          ) : null}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </ScrollArea>
          ) : (
            <Empty className="surface-workspace-inset min-h-[280px] border bg-card">
              <EmptyMedia>
                <FileText />
              </EmptyMedia>
              <div>
                <EmptyTitle>No transcripts today</EmptyTitle>
                <EmptyDescription>Finished transcriptions will show up here as a quick feed.</EmptyDescription>
              </div>
            </Empty>
          )}
        </section>
      </div>

      <aside className="min-w-0 space-y-4">
        <Card className="surface-workspace-inset py-0">
          <CardHeader className="p-4">
            <CardTitle className="text-base">At a glance</CardTitle>
            <CardDescription>Local activity on this device.</CardDescription>
          </CardHeader>
          <CardContent className="p-4 pt-0">
            <p className="text-sm leading-6 text-muted-foreground">{glanceSummary}</p>
          </CardContent>
        </Card>

        {queueCount && !running ? (
          <Card className="surface-workspace-inset py-0">
            <CardContent className="p-4">
              <p className="text-sm font-medium">
                <span className="tabular-nums">{queueCount}</span> file{queueCount === 1 ? "" : "s"} waiting to
                transcribe
              </p>
              <Button className="mt-3 w-full" onClick={onOpenTranscribe} type="button" variant="secondary">
                Continue in transcribe
              </Button>
            </CardContent>
          </Card>
        ) : null}
      </aside>
    </div>
  );
}
