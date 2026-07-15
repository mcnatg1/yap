import { shouldSearchTranscriptBodies, type PreviewSearchFailureState } from "@/lib/history-preview-loader";

const noUnavailablePreviewPaths: ReadonlySet<string> = new Set();

export type HistorySearchFailureState = PreviewSearchFailureState;

export function normalizeHistorySearchQuery(query: string) {
  return query.trim().toLowerCase();
}

export function historySearchFailurePathsForQuery(
  failure: HistorySearchFailureState,
  query: string,
) {
  return shouldSearchTranscriptBodies(query)
    ? failure.paths
    : noUnavailablePreviewPaths;
}

export function projectHistorySearchDisplay({
  hasResults,
  indexingBodies,
  hasUnavailableBodies = false,
}: {
  hasResults: boolean;
  indexingBodies: boolean;
  hasUnavailableBodies?: boolean;
}): "results" | "indexing" | "unavailable" | "empty" {
  if (hasResults) return "results";
  if (indexingBodies) return "indexing";
  return hasUnavailableBodies ? "unavailable" : "empty";
}

export function isHistoryBodySearchPending({
  cachedOutputPaths,
  terminalOutputPaths = new Set<string>(),
  hasPreviewLoader,
  outputPaths,
  query,
}: {
  cachedOutputPaths: ReadonlySet<string>;
  terminalOutputPaths?: ReadonlySet<string>;
  hasPreviewLoader: boolean;
  outputPaths: readonly string[];
  query: string;
}) {
  return hasPreviewLoader
    && shouldSearchTranscriptBodies(query)
    && outputPaths.some(
      (outputPath) => !cachedOutputPaths.has(outputPath) && !terminalOutputPaths.has(outputPath),
    );
}
