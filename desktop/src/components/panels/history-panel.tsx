import { useMemo, useState, type MouseEvent } from "react";
import { Copy, FileText, FolderOpen, MoreHorizontal, Search, Trash2 } from "lucide-react";

import { HistoryEntryPreview } from "@/components/panels/history-entry-preview";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { TranscriptHistoryEntry } from "@/history";
import { formatHistoryTime, groupHistoryByDay } from "@/lib/app-types";
import { cn } from "@/lib/utils";

function HistoryActionMenu({
  entry,
  onCopy,
  onOpen,
  onPreview,
  onRemove,
  onReveal,
}: {
  entry: TranscriptHistoryEntry;
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  onRemove: (outputPath: string) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
}) {
  const [confirmRemove, setConfirmRemove] = useState(false);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={`Actions for ${entry.name}`}
            onClick={(event) => event.stopPropagation()}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" onClick={(event) => event.stopPropagation()}>
          <DropdownMenuLabel>Transcript</DropdownMenuLabel>
          <DropdownMenuGroup>
            <DropdownMenuItem onSelect={() => onPreview(entry)}>
              <FileText />
              Preview
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onCopy(entry)}>
              <Copy />
              Copy transcript
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onOpen(entry)}>
              <FileText />
              Open file
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => onReveal(entry)}>
              <FolderOpen />
              Reveal in Explorer
            </DropdownMenuItem>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onSelect={(event) => {
              event.preventDefault();
              setConfirmRemove(true);
            }}
            variant="destructive"
          >
            <Trash2 />
            Remove from history
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <AlertDialog onOpenChange={setConfirmRemove} open={confirmRemove}>
        <AlertDialogContent onClick={(event) => event.stopPropagation()}>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove from history?</AlertDialogTitle>
            <AlertDialogDescription>
              This removes {entry.name} from your Yap history list. The transcript file on disk stays
              untouched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-white hover:bg-destructive/90 focus-visible:ring-destructive/20"
              onClick={() => onRemove(entry.outputPath)}
            >
              Remove from history
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

export function HistoryPanel({
  entries,
  onCopy,
  onLoadPreviewText,
  onOpen,
  onOpenHelp,
  onPreview,
  onRemove,
  onReveal,
  onSelect,
  selectedOutputPath,
}: {
  entries: TranscriptHistoryEntry[];
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onLoadPreviewText?: (entry: TranscriptHistoryEntry) => Promise<string>;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onOpenHelp?: () => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  onRemove: (outputPath: string) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
  onSelect: (entry: TranscriptHistoryEntry, origin?: DOMRect) => void;
  selectedOutputPath?: string;
}) {
  const [searchFilter, setSearchFilter] = useState("");

  const visibleGroups = useMemo(() => {
    const query = searchFilter.trim().toLowerCase();
    const filtered = query
      ? entries.filter((entry) => `${entry.name} ${entry.sourcePath}`.toLowerCase().includes(query))
      : entries;

    return groupHistoryByDay(filtered);
  }, [entries, searchFilter]);

  return (
    <Card className="surface-workspace-inset min-w-0 bg-card py-0">
      <CardHeader className="p-4 sm:p-5">
        <div className="min-w-0">
          <CardTitle className="flex items-center gap-2 text-xl">
            Recordings
            <Badge className="tabular-nums" variant="secondary">
              {entries.length}
            </Badge>
          </CardTitle>
          <CardDescription>
            Saved recordings and transcripts stay on this computer. Select a row to review it.
          </CardDescription>
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 p-4 sm:p-5">
        {entries.length ? (
          <>
            <InputGroup>
              <InputGroupInput
                aria-label="Search recordings"
                onChange={(event) => setSearchFilter(event.target.value)}
                placeholder="Search recordings"
                type="search"
                value={searchFilter}
              />
              <InputGroupAddon align="inline-end">
                <Search />
              </InputGroupAddon>
            </InputGroup>

            <ScrollArea className="h-[min(520px,calc(100vh-280px))] pr-3">
              {visibleGroups.length ? (
                <div className="flex flex-col gap-6">
                  {visibleGroups.map((group) => (
                    <section key={group.key}>
                      <h3 className="mb-2 text-xs font-semibold text-muted-foreground">{group.label}</h3>
                      <Table>
                        <TableHeader>
                          <TableRow className="hover:bg-transparent">
                            <TableHead>Transcript</TableHead>
                            <TableHead className="w-20 text-right">Time</TableHead>
                            <TableHead className="w-[4.5rem]">
                              <span className="sr-only">Actions</span>
                            </TableHead>
                          </TableRow>
                        </TableHeader>
                        <TableBody>
                          {group.entries.map((entry) => {
                            const selected = entry.outputPath === selectedOutputPath;

                            function selectEntry(event: MouseEvent<HTMLElement>) {
                              const row = event.currentTarget.closest("[data-history-entry-row]");
                              onSelect(entry, row?.getBoundingClientRect());
                            }

                            return (
                              <TableRow
                                key={entry.outputPath}
                                aria-current={selected ? "true" : undefined}
                                className={cn(
                                  selected && "border-primary/30 bg-[var(--primary-soft)]/40 hover:bg-[var(--primary-soft)]/40",
                                )}
                                data-state={selected ? "selected" : undefined}
                                data-history-entry-row
                              >
                                <TableCell
                                  className="max-w-0 cursor-pointer whitespace-normal"
                                  onClick={selectEntry}
                                >
                                  <div className="flex min-w-0 items-center gap-2">
                                    <FileText className="size-4 shrink-0 text-muted-foreground" />
                                    <HistoryEntryPreview
                                      entry={entry}
                                      onLoadPreviewText={onLoadPreviewText}
                                      onReview={(origin) => onSelect(entry, origin)}
                                    />
                                  </div>
                                </TableCell>
                                <TableCell
                                  className="cursor-pointer text-right text-xs tabular-nums text-muted-foreground"
                                  onClick={selectEntry}
                                >
                                  {formatHistoryTime(entry.createdAt)}
                                </TableCell>
                                <TableCell className="text-right">
                                  <div className="flex items-center justify-end gap-0.5">
                                    <Tooltip>
                                      <TooltipTrigger asChild>
                                        <Button
                                          aria-label={`Copy transcript for ${entry.name}`}
                                          onClick={(event) => {
                                            event.stopPropagation();
                                            onCopy(entry);
                                          }}
                                          size="icon-xs"
                                          title="Copy"
                                          type="button"
                                          variant="ghost"
                                        >
                                          <Copy />
                                        </Button>
                                      </TooltipTrigger>
                                      <TooltipContent>Copy</TooltipContent>
                                    </Tooltip>
                                    <HistoryActionMenu
                                      entry={entry}
                                      onCopy={onCopy}
                                      onOpen={onOpen}
                                      onPreview={onPreview}
                                      onRemove={onRemove}
                                      onReveal={onReveal}
                                    />
                                  </div>
                                </TableCell>
                              </TableRow>
                            );
                          })}
                        </TableBody>
                      </Table>
                    </section>
                  ))}
                </div>
              ) : (
                <p className="py-8 text-center text-sm text-muted-foreground">No recordings match that search.</p>
              )}
            </ScrollArea>
          </>
        ) : (
          <Empty className="min-h-[260px]">
            <EmptyMedia>
              <FileText />
            </EmptyMedia>
            <div>
              <EmptyTitle>No recordings yet</EmptyTitle>
              <EmptyDescription>Finished transcriptions will appear here, grouped by day.</EmptyDescription>
              {onOpenHelp ? (
                <Button
                  className="mt-2 h-auto px-0 text-muted-foreground"
                  onClick={onOpenHelp}
                  size="sm"
                  type="button"
                  variant="link"
                >
                  Learn more
                </Button>
              ) : null}
            </div>
          </Empty>
        )}
      </CardContent>
    </Card>
  );
}
