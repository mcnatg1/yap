import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { Copy } from "@phosphor-icons/react/Copy";
import { ArrowClockwise as Recover } from "@phosphor-icons/react/ArrowClockwise";
import { EyeSlash } from "@phosphor-icons/react/EyeSlash";
import { FileText } from "@phosphor-icons/react/FileText";
import { FolderOpen } from "@phosphor-icons/react/FolderOpen";
import { DotsThree as MoreHorizontal } from "@phosphor-icons/react/DotsThree";
import { MagnifyingGlass as Search } from "@phosphor-icons/react/MagnifyingGlass";
import { Trash as Trash2 } from "@phosphor-icons/react/Trash";

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
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
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
  TableRow,
} from "@/components/ui/table";
import {
  canDeleteTranscriptHistoryEntry,
  isRecoverableTranscriptHistoryEntry,
  maxTranscriptHistoryEntries,
  type TranscriptHistoryEntry,
} from "@/history";
import { formatHistoryTime, groupHistoryByDay } from "@/lib/app-types";
import { historyRenderWindowSize, renderHistoryWindow } from "@/lib/history-render-window";
import {
  createPreviewSearchGenerationGuard,
  createPreviewTextLoader,
  previewSearchEntries,
  shouldSearchTranscriptBodies,
} from "@/lib/history-preview-loader";
import { rememberText } from "@/lib/text-cache";
import { cn } from "@/lib/utils";

const maxHistoryPreviewCacheEntries = maxTranscriptHistoryEntries;

export function projectHistorySearchDisplay({
  hasResults,
  indexingBodies,
}: {
  hasResults: boolean;
  indexingBodies: boolean;
}): "results" | "indexing" | "empty" {
  if (hasResults) return "results";
  return indexingBodies ? "indexing" : "empty";
}

export function isHistoryBodySearchPending({
  cachedOutputPaths,
  hasPreviewLoader,
  outputPaths,
  query,
}: {
  cachedOutputPaths: ReadonlySet<string>;
  hasPreviewLoader: boolean;
  outputPaths: readonly string[];
  query: string;
}) {
  return hasPreviewLoader
    && shouldSearchTranscriptBodies(query)
    && outputPaths.some((outputPath) => !cachedOutputPaths.has(outputPath));
}

function HistoryActionMenu({
  entry,
  onCopy,
  onDelete,
  onDeleteRecoverable,
  onHide,
  onOpen,
  onPreview,
  onRecover,
  onReveal,
}: {
  entry: TranscriptHistoryEntry;
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onDelete: (entry: TranscriptHistoryEntry) => void;
  onDeleteRecoverable: (entry: TranscriptHistoryEntry) => void;
  onHide: (outputPath: string) => void;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  onRecover: (entry: TranscriptHistoryEntry) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
}) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const canDelete = canDeleteTranscriptHistoryEntry(entry);
  const recoverable = isRecoverableTranscriptHistoryEntry(entry);

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
          <DropdownMenuLabel>{recoverable ? "Partial" : "Transcript"}</DropdownMenuLabel>
          {recoverable ? (
            <DropdownMenuGroup>
              <DropdownMenuItem onSelect={() => onRecover(entry)}>
                <Recover />
                Recover
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => onDeleteRecoverable(entry)} variant="destructive">
                <Trash2 />
                Delete
              </DropdownMenuItem>
            </DropdownMenuGroup>
          ) : (
            <>
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
              <DropdownMenuItem onSelect={() => onHide(entry.outputPath)}>
                <EyeSlash />
                Hide
              </DropdownMenuItem>
            </>
          )}
          {!recoverable && canDelete ? (
            <DropdownMenuItem
              onSelect={(event) => {
                event.preventDefault();
                setConfirmDelete(true);
              }}
              variant="destructive"
            >
              <Trash2 />
              Delete
            </DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      <AlertDialog onOpenChange={setConfirmDelete} open={confirmDelete}>
        <AlertDialogContent onClick={(event) => event.stopPropagation()}>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete from device?</AlertDialogTitle>
            <AlertDialogDescription>
              This deletes the saved transcript. If the recording was captured by Yap, that audio file
              is deleted too.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-white hover:bg-destructive/90 focus-visible:ring-destructive/20"
              onClick={() => onDelete(entry)}
            >
              Delete
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
  onDelete,
  onDeleteRecoverable,
  onHide,
  onLoadPreviewText,
  onOpen,
  onOpenHelp,
  onPreview,
  onRecover,
  onReveal,
  onSelect,
  selectedOutputPath,
}: {
  entries: TranscriptHistoryEntry[];
  onCopy: (entry: TranscriptHistoryEntry) => void;
  onDelete: (entry: TranscriptHistoryEntry) => void;
  onDeleteRecoverable: (entry: TranscriptHistoryEntry) => void;
  onHide: (outputPath: string) => void;
  onLoadPreviewText?: (entry: TranscriptHistoryEntry) => Promise<string>;
  onOpen: (entry: TranscriptHistoryEntry) => void;
  onOpenHelp?: () => void;
  onPreview: (entry: TranscriptHistoryEntry) => void;
  onRecover: (entry: TranscriptHistoryEntry) => void;
  onReveal: (entry: TranscriptHistoryEntry) => void;
  onSelect: (entry: TranscriptHistoryEntry, origin?: DOMRect) => void;
  selectedOutputPath?: string;
}) {
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchFilter, setSearchFilter] = useState("");
  const [renderLimit, setRenderLimit] = useState(historyRenderWindowSize);
  const [previewTextByPath, setPreviewTextByPath] = useState<Record<string, string>>({});
  const previewTextByPathRef = useRef(previewTextByPath);
  const previewLoaderRef = useRef(createPreviewTextLoader());
  const previewSearchGenerationRef = useRef(createPreviewSearchGenerationGuard());

  useEffect(() => {
    previewTextByPathRef.current = previewTextByPath;
  }, [previewTextByPath]);

  const loadPreviewText = useCallback(async (entry: TranscriptHistoryEntry) => {
    return previewLoaderRef.current.load(
      entry,
      previewTextByPathRef.current,
      onLoadPreviewText,
      (outputPath, text) => {
        setPreviewTextByPath((current) =>
          current[outputPath] === undefined
            ? rememberText(current, outputPath, text, maxHistoryPreviewCacheEntries)
            : current,
        );
      },
    );
  }, [onLoadPreviewText]);

  const searchableEntries = useMemo(() => previewSearchEntries(entries), [entries]);
  const searchableOutputPaths = useMemo(
    () => searchableEntries.map((entry) => entry.outputPath),
    [searchableEntries],
  );
  const cachedOutputPaths = useMemo(
    () => new Set(Object.keys(previewTextByPath)),
    [previewTextByPath],
  );
  const indexingBodies = isHistoryBodySearchPending({
    cachedOutputPaths,
    hasPreviewLoader: Boolean(onLoadPreviewText),
    outputPaths: searchableOutputPaths,
    query: searchFilter,
  });

  useEffect(() => {
    if (!indexingBodies || !onLoadPreviewText) return;

    let cancelled = false;
    const generation = previewSearchGenerationRef.current.begin();
    void (async () => {
      for (const entry of searchableEntries) {
        if (cancelled) break;
        if (previewTextByPathRef.current[entry.outputPath] !== undefined) continue;
        try {
          await previewLoaderRef.current.load(
            entry,
            previewTextByPathRef.current,
            onLoadPreviewText,
            (outputPath, text) => {
              if (cancelled || !previewSearchGenerationRef.current.isCurrent(generation)) return;
              setPreviewTextByPath((current) =>
                current[outputPath] === undefined
                  ? rememberText(current, outputPath, text, maxHistoryPreviewCacheEntries)
                  : current,
              );
            },
          );
        } catch {
          // Keep search indexing the rest of history when one transcript moved or is unreadable.
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [indexingBodies, onLoadPreviewText, searchFilter, searchableEntries]);

  useEffect(() => {
    setRenderLimit(historyRenderWindowSize);
  }, [entries, searchFilter]);

  const filteredEntries = useMemo(() => {
    const query = searchFilter.trim().toLowerCase();
    const includePreviewText = shouldSearchTranscriptBodies(query);
    return query
      ? searchableEntries.filter((entry) =>
          `${entry.name} ${entry.sourcePath} ${includePreviewText ? previewTextByPath[entry.outputPath] ?? "" : ""}`
            .toLowerCase()
            .includes(query),
        )
      : searchableEntries;
  }, [previewTextByPath, searchFilter, searchableEntries]);

  const historyWindow = useMemo(
    () => renderHistoryWindow(filteredEntries, renderLimit),
    [filteredEntries, renderLimit],
  );

  const visibleGroups = useMemo(
    () => groupHistoryByDay(historyWindow.visibleEntries),
    [historyWindow.visibleEntries],
  );
  const searchDisplay = projectHistorySearchDisplay({
    hasResults: visibleGroups.length > 0,
    indexingBodies,
  });

  return (
    <Card className="surface-workspace-inset min-w-0 bg-card py-0">
      <CardContent className="grid gap-4 p-4 sm:p-5">
        {entries.length ? (
          <>
            <div className="flex items-center justify-end">
              {searchOpen ? (
                <div className="flex h-9 w-64 max-w-full items-center gap-3 text-muted-foreground">
                  <Search className="size-5 shrink-0 text-muted-foreground/70" weight="regular" />
                  <input
                    aria-label="Search past transcripts"
                    autoFocus
                    autoComplete="off"
                    autoCorrect="off"
                    spellCheck={false}
                    className="h-full min-w-0 flex-1 border-0 bg-transparent p-0 text-base font-normal text-foreground outline-none placeholder:text-muted-foreground/85 focus-visible:outline-none"
                    placeholder="Search past transcripts"
                    type="text"
                    value={searchFilter}
                    onChange={(event) => setSearchFilter(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Escape") {
                        setSearchFilter("");
                        setSearchOpen(false);
                      }
                    }}
                  />
                </div>
              ) : (
                <Button
                  aria-label="Search past transcripts"
                  onClick={() => setSearchOpen(true)}
                  size="icon-xs"
                  type="button"
                  variant="ghost"
                >
                  <Search />
                </Button>
              )}
            </div>
            <ScrollArea className="h-[min(620px,calc(100vh-230px))] pr-3">
              {searchDisplay === "results" ? (
                <div className="flex flex-col gap-6">
                  {visibleGroups.map((group) => (
                    <section key={group.key}>
                      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">{group.label}</h3>
                      <Table>
                        <TableBody>
                          {group.entries.map((entry) => {
                            const selected = entry.outputPath === selectedOutputPath;
                            const recoverable = isRecoverableTranscriptHistoryEntry(entry);

                            function selectEntry(event: MouseEvent<HTMLElement>) {
                              if (recoverable) return;
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
                                  className={cn(
                                    "w-24 align-top text-xs tabular-nums text-muted-foreground",
                                    !recoverable && "cursor-pointer",
                                  )}
                                  onClick={recoverable ? undefined : selectEntry}
                                >
                                  {formatHistoryTime(entry.createdAt)}
                                </TableCell>
                                <TableCell
                                  className={cn("max-w-0 whitespace-normal align-top", !recoverable && "cursor-pointer")}
                                  onClick={recoverable ? undefined : selectEntry}
                                >
                                  <div className="flex min-w-0 items-start gap-2">
                                    {recoverable ? (
                                      <span className="shrink-0 text-xs font-medium text-muted-foreground">Partial</span>
                                    ) : (
                                      <HistoryEntryPreview
                                        entry={entry}
                                        onLoadPreviewText={loadPreviewText}
                                        onReview={(origin) => onSelect(entry, origin)}
                                      />
                                    )}
                                  </div>
                                </TableCell>
                                <TableCell className="w-[4.5rem] align-top text-right">
                                  <div className="flex items-center justify-end gap-0.5">
                                    {!recoverable ? (
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
                                    ) : null}
                                    <HistoryActionMenu
                                      entry={entry}
                                      onCopy={onCopy}
                              onDelete={onDelete}
                              onDeleteRecoverable={onDeleteRecoverable}
                                      onHide={onHide}
                                      onOpen={onOpen}
                              onPreview={onPreview}
                              onRecover={onRecover}
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
                  {historyWindow.hiddenCount ? (
                    <Button
                      className="self-center"
                      onClick={() => setRenderLimit(historyWindow.nextLimit)}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      Show older
                    </Button>
                  ) : null}
                </div>
              ) : searchDisplay === "indexing" ? (
                <div
                  aria-live="polite"
                  className="flex items-center justify-center py-8 text-sm text-muted-foreground"
                >
                  Searching transcript text...
                </div>
              ) : (
                <div className="flex flex-col items-center gap-3 py-8 text-sm text-muted-foreground">
                  <p>No recordings match that search.</p>
                  <Button
                    onClick={() => {
                      setSearchFilter("");
                      setSearchOpen(false);
                    }}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    Clear search
                  </Button>
                </div>
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
