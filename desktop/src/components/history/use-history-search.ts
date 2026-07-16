import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  historySearchFailurePathsForQuery,
  isHistoryBodySearchPending,
  normalizeHistorySearchQuery,
  type HistorySearchFailureState,
} from "@/components/history/history-search";
import { maxTranscriptHistoryEntries, type TranscriptHistoryEntry } from "@/history-model";
import {
  createPreviewSearchGenerationGuard,
  createPreviewSearchLoader,
  createPreviewTextLoader,
  mergePreviewSearchFailures,
  previewSearchEntries,
  prunePreviewSearchFailures,
  shouldSearchTranscriptBodies,
} from "@/lib/history-preview-loader";
import { pruneTextCache, rememberText, rememberTexts } from "@/lib/text-cache";

const maxHistoryPreviewCacheEntries = maxTranscriptHistoryEntries;
const maxHistoryPreviewCharsPerEntry = 2_000;
const maxHistoryPreviewCacheChars =
  maxHistoryPreviewCacheEntries * maxHistoryPreviewCharsPerEntry;

type HistoryPreviewState = {
  previewTextByPath: Record<string, string>;
  searchFailures: HistorySearchFailureState;
};

export function useHistorySearch({
  entries,
  onLoadPreviewText,
  query,
}: {
  entries: TranscriptHistoryEntry[];
  onLoadPreviewText?: (entry: TranscriptHistoryEntry) => Promise<string>;
  query: string;
}) {
  const [retryGeneration, setRetryGeneration] = useState(0);
  const [previewState, setPreviewState] = useState<HistoryPreviewState>(() => ({
    previewTextByPath: {},
    searchFailures: { paths: new Set() },
  }));
  const { previewTextByPath, searchFailures } = previewState;
  const previewTextByPathRef = useRef(previewTextByPath);
  const previewLoaderRef = useRef(createPreviewTextLoader({
    maxCharsPerEntry: maxHistoryPreviewCharsPerEntry,
    maxEntries: maxHistoryPreviewCacheEntries,
    maxTotalChars: maxHistoryPreviewCacheChars,
  }));
  const previewSearchLoaderRef = useRef(createPreviewSearchLoader());
  const previewSearchGenerationRef = useRef(createPreviewSearchGenerationGuard());

  useEffect(() => {
    previewTextByPathRef.current = previewTextByPath;
  }, [previewTextByPath]);

  const searchableEntries = useMemo(() => previewSearchEntries(entries), [entries]);
  const searchableOutputPaths = useMemo(
    () => searchableEntries.map((entry) => entry.outputPath),
    [searchableEntries],
  );
  const searchableOutputPathSet = useMemo(
    () => new Set(searchableOutputPaths),
    [searchableOutputPaths],
  );
  const searchableOutputPathSetRef = useRef(searchableOutputPathSet);
  searchableOutputPathSetRef.current = searchableOutputPathSet;

  const loadPreviewText = useCallback(async (entry: TranscriptHistoryEntry) => {
    return previewLoaderRef.current.load(
      entry,
      previewTextByPathRef.current,
      onLoadPreviewText,
      (outputPath, text) => {
        if (!searchableOutputPathSetRef.current.has(outputPath)) return;
        setPreviewState((current) => {
          const hasFailure = current.searchFailures.paths.has(outputPath);
          const hasText = current.previewTextByPath[outputPath] !== undefined;
          if (!hasFailure && hasText) return current;

          const paths = hasFailure
            ? new Set([...current.searchFailures.paths].filter((path) => path !== outputPath))
            : current.searchFailures.paths;
          return {
            previewTextByPath: hasText
              ? current.previewTextByPath
              : rememberText(
                  current.previewTextByPath,
                  outputPath,
                  text,
                  maxHistoryPreviewCacheEntries,
                  maxHistoryPreviewCharsPerEntry,
                  maxHistoryPreviewCacheChars,
                ),
            searchFailures: hasFailure
              ? { ...current.searchFailures, paths }
              : current.searchFailures,
          };
        });
      },
    );
  }, [onLoadPreviewText]);

  const retrySearchFailures = useCallback(() => {
    previewLoaderRef.current.retryFailures();
    setRetryGeneration((current) => current + 1);
    setPreviewState((current) => ({
      ...current,
      searchFailures: { paths: new Set() },
    }));
  }, []);
  const cachedOutputPaths = useMemo(
    () => new Set(Object.keys(previewTextByPath)),
    [previewTextByPath],
  );
  const normalizedQuery = normalizeHistorySearchQuery(query);
  const unavailablePreviewPaths = historySearchFailurePathsForQuery(
    searchFailures,
    normalizedQuery,
  );
  const indexingBodies = isHistoryBodySearchPending({
    cachedOutputPaths,
    hasPreviewLoader: Boolean(onLoadPreviewText),
    outputPaths: searchableOutputPaths,
    query,
    terminalOutputPaths: unavailablePreviewPaths,
  });

  useEffect(() => {
    previewLoaderRef.current.prune(searchableOutputPathSet);
    setPreviewState((current) => {
      const previewText = pruneTextCache(
        current.previewTextByPath,
        searchableOutputPathSet,
      );
      const failures = prunePreviewSearchFailures(
        current.searchFailures,
        searchableOutputPathSet,
      );
      if (
        previewText === current.previewTextByPath
        && failures === current.searchFailures
      ) {
        return current;
      }
      return {
        previewTextByPath: previewText,
        searchFailures: failures,
      };
    });
  }, [searchableOutputPathSet]);

  useEffect(() => {
    if (!indexingBodies || !onLoadPreviewText) return;

    const controller = new AbortController();
    const generation = previewSearchGenerationRef.current.begin();
    const pendingEntries = searchableEntries.filter(
      (entry) => previewTextByPathRef.current[entry.outputPath] === undefined,
    );

    void previewSearchLoaderRef.current.load({
      entries: pendingEntries,
      loadText: (entry) =>
        previewLoaderRef.current.load(
          entry,
          previewTextByPathRef.current,
          onLoadPreviewText,
          () => undefined,
        ),
      onBatch: (batch) => {
        if (
          controller.signal.aborted
          || !previewSearchGenerationRef.current.isCurrent(generation)
        ) {
          return;
        }

        setPreviewState((current) => {
          if (
            controller.signal.aborted
            || !previewSearchGenerationRef.current.isCurrent(generation)
          ) {
            return current;
          }

          const retainedPreviewText = pruneTextCache(
            current.previewTextByPath,
            searchableOutputPathSet,
          );
          const loaded = batch.loaded.filter(
            ({ outputPath }) =>
              searchableOutputPathSet.has(outputPath)
              && retainedPreviewText[outputPath] === undefined,
          );
          const failedOutputPaths = batch.failedOutputPaths.filter(
            (outputPath) => retainedPreviewText[outputPath] === undefined,
          );
          const nextPreviewText = loaded.length > 0
            ? rememberTexts(
                retainedPreviewText,
                loaded.map(({ outputPath, text }) => [outputPath, text]),
                maxHistoryPreviewCacheEntries,
                maxHistoryPreviewCharsPerEntry,
                maxHistoryPreviewCacheChars,
              )
            : retainedPreviewText;
          const nextSearchFailures = mergePreviewSearchFailures({
            current: current.searchFailures,
            failedOutputPaths,
            loadedOutputPaths: batch.loaded.map(({ outputPath }) => outputPath),
            visibleOutputPaths: searchableOutputPathSet,
          });

          if (
            nextPreviewText === current.previewTextByPath
            && nextSearchFailures === current.searchFailures
          ) {
            return current;
          }

          return {
            previewTextByPath: nextPreviewText,
            searchFailures: nextSearchFailures,
          };
        });
      },
      signal: controller.signal,
    });

    return () => {
      controller.abort();
    };
  }, [
    indexingBodies,
    normalizedQuery,
    onLoadPreviewText,
    retryGeneration,
    searchableEntries,
    searchableOutputPathSet,
  ]);

  const filteredEntries = useMemo(() => {
    const includePreviewText = shouldSearchTranscriptBodies(normalizedQuery);
    return normalizedQuery
      ? searchableEntries.filter((entry) =>
          `${entry.name} ${entry.sourcePath} ${includePreviewText ? previewTextByPath[entry.outputPath] ?? "" : ""}`
            .toLowerCase()
            .includes(normalizedQuery),
        )
      : searchableEntries;
  }, [normalizedQuery, previewTextByPath, searchableEntries]);

  return {
    filteredEntries,
    indexingBodies,
    loadPreviewText,
    retrySearchFailures,
    unavailablePreviewPaths,
  };
}
