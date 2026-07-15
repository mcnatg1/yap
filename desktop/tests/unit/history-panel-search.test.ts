import { describe, expect, it } from "vitest";

import {
  historySearchFailurePathsForQuery,
  isHistoryBodySearchPending,
  projectHistorySearchDisplay,
} from "@/components/history/history-search";

describe("history panel transcript search", () => {
  it("settles failed preview reads as unavailable instead of pending forever", () => {
    expect(isHistoryBodySearchPending({
      cachedOutputPaths: new Set(),
      hasPreviewLoader: true,
      outputPaths: ["failed.txt"],
      query: "needle",
      terminalOutputPaths: new Set(["failed.txt"]),
    })).toBe(false);
    expect(projectHistorySearchDisplay({
      hasResults: false,
      hasUnavailableBodies: true,
      indexingBodies: false,
    })).toBe("unavailable");
  });

  it("keeps terminal preview failures across body-search query edits", () => {
    const failure = {
      paths: new Set(["failed.txt"]),
    };

    expect(historySearchFailurePathsForQuery(failure, " NEEDLE "))
      .toEqual(new Set(["failed.txt"]));
    expect(historySearchFailurePathsForQuery(failure, "n")).toEqual(new Set());
    expect(historySearchFailurePathsForQuery(failure, "different"))
      .toEqual(new Set(["failed.txt"]));
    expect(isHistoryBodySearchPending({
      cachedOutputPaths: new Set(),
      hasPreviewLoader: true,
      outputPaths: ["failed.txt"],
      query: "different",
      terminalOutputPaths: historySearchFailurePathsForQuery(failure, "different"),
    })).toBe(false);
  });

  it("keeps visible results while body indexing or failures make them incomplete", () => {
    expect(projectHistorySearchDisplay({ hasResults: true, indexingBodies: true }))
      .toBe("results");
    expect(projectHistorySearchDisplay({
      hasResults: true,
      hasUnavailableBodies: true,
      indexingBodies: false,
    })).toBe("results");
  });
});
