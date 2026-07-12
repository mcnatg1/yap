import { describe, expect, it } from "vitest";

import {
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
});
