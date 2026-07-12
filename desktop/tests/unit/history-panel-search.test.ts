import { describe, expect, it } from "vitest";

import {
  historySearchFailurePathsForQuery,
  isHistoryBodySearchPending,
  projectHistorySearchDisplay,
} from "@/components/panels/history-panel";

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

  it("scopes terminal preview failures to the body-search query that produced them", () => {
    const failure = {
      paths: new Set(["failed.txt"]),
      query: "needle",
    };

    expect(historySearchFailurePathsForQuery(failure, " NEEDLE "))
      .toEqual(new Set(["failed.txt"]));
    expect(historySearchFailurePathsForQuery(failure, "n")).toEqual(new Set());
    expect(historySearchFailurePathsForQuery(failure, "different")).toEqual(new Set());
    expect(isHistoryBodySearchPending({
      cachedOutputPaths: new Set(),
      hasPreviewLoader: true,
      outputPaths: ["failed.txt"],
      query: "different",
      terminalOutputPaths: historySearchFailurePathsForQuery(failure, "different"),
    })).toBe(true);
  });
});
