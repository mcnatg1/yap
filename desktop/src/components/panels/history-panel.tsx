import { FileText } from "@phosphor-icons/react/FileText";
import { MagnifyingGlass as Search } from "@phosphor-icons/react/MagnifyingGlass";
import { useEffect, useMemo, useState } from "react";

import type { HistoryEntryActions, HistoryPanelProps } from "@/components/history/history-panel-contract";
import { HistoryResults } from "@/components/history/history-results";
import { projectHistorySearchDisplay } from "@/components/history/history-search";
import { useHistorySearch } from "@/components/history/use-history-search";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Empty, EmptyDescription, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { ScrollArea } from "@/components/ui/scroll-area";
import { groupHistoryByDay } from "@/lib/app-types";
import { historyRenderWindowSize, renderHistoryWindow } from "@/lib/history-render-window";

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
}: HistoryPanelProps) {
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchFilter, setSearchFilter] = useState("");
  const [renderLimit, setRenderLimit] = useState(historyRenderWindowSize);
  const {
    filteredEntries,
    indexingBodies,
    loadPreviewText,
    retrySearchFailures,
    unavailablePreviewPaths,
  } = useHistorySearch({
    entries,
    onLoadPreviewText,
    query: searchFilter,
  });

  useEffect(() => {
    setRenderLimit(historyRenderWindowSize);
  }, [entries, searchFilter]);

  const historyWindow = useMemo(
    () => renderHistoryWindow(filteredEntries, renderLimit),
    [filteredEntries, renderLimit],
  );
  const visibleGroups = useMemo(
    () => groupHistoryByDay(historyWindow.visibleEntries),
    [historyWindow.visibleEntries],
  );
  const searchDisplay = projectHistorySearchDisplay({
    hasUnavailableBodies: unavailablePreviewPaths.size > 0,
    hasResults: visibleGroups.length > 0,
    indexingBodies,
  });
  const actions: HistoryEntryActions = {
    onCopy,
    onDelete,
    onDeleteRecoverable,
    onHide,
    onOpen,
    onPreview,
    onRecover,
    onReveal,
  };

  function clearSearch() {
    setSearchFilter("");
    setSearchOpen(false);
  }

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
                    autoComplete="off"
                    autoCorrect="off"
                    autoFocus
                    className="h-full min-w-0 flex-1 border-0 bg-transparent p-0 text-base font-normal text-foreground outline-none placeholder:text-muted-foreground/85 focus-visible:outline-none"
                    onChange={(event) => setSearchFilter(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Escape") clearSearch();
                    }}
                    placeholder="Search past transcripts"
                    spellCheck={false}
                    type="text"
                    value={searchFilter}
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
            <ScrollArea
              aria-busy={indexingBodies || undefined}
              className="h-[min(620px,calc(100vh-230px))] pr-3"
            >
              {searchDisplay === "results" ? (
                <HistoryResults
                  actions={actions}
                  groups={visibleGroups}
                  hiddenCount={historyWindow.hiddenCount}
                  indexingBodies={indexingBodies}
                  loadPreviewText={loadPreviewText}
                  nextLimit={historyWindow.nextLimit}
                  onRetrySearch={retrySearchFailures}
                  onSelect={onSelect}
                  onShowOlder={setRenderLimit}
                  selectedOutputPath={selectedOutputPath}
                  unavailablePreviewPaths={unavailablePreviewPaths}
                />
              ) : searchDisplay === "indexing" ? (
                <div
                  aria-live="polite"
                  className="flex items-center justify-center py-8 text-sm text-muted-foreground"
                >
                  Searching transcript text...
                </div>
              ) : searchDisplay === "unavailable" ? (
                <div className="flex flex-col items-center gap-3 py-8 text-sm text-muted-foreground">
                  <p>Some transcripts are unavailable. No available recordings match that search.</p>
                  <Button
                    onClick={retrySearchFailures}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    Retry search
                  </Button>
                </div>
              ) : (
                <div className="flex flex-col items-center gap-3 py-8 text-sm text-muted-foreground">
                  <p>No recordings match that search.</p>
                  <Button onClick={clearSearch} size="sm" type="button" variant="outline">
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
              <EmptyDescription>
                Finished transcriptions will appear here, grouped by day.
              </EmptyDescription>
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
