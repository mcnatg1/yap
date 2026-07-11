import { describe, expect, it } from "vitest";

import { projectAppModalState } from "@/components/panels/app-sheets";
import { projectHistorySearchDisplay } from "@/components/panels/history-panel";
import { isDevelopmentPolishAvailable, isPolishDraftCurrent } from "@/polish";

describe("product surface contracts", () => {
  it("does not report an empty history while transcript bodies are indexing", () => {
    expect(projectHistorySearchDisplay({ hasResults: false, indexingBodies: true }))
      .toBe("indexing");
    expect(projectHistorySearchDisplay({ hasResults: true, indexingBodies: true }))
      .toBe("results");
    expect(projectHistorySearchDisplay({ hasResults: false, indexingBodies: false }))
      .toBe("empty");
  });

  it("gives Settings and Help one mutually exclusive modal owner", () => {
    expect(projectAppModalState("settings")).toEqual({ detailsOpen: true, helpOpen: false });
    expect(projectAppModalState("help")).toEqual({ detailsOpen: false, helpOpen: true });
    expect(projectAppModalState(null)).toEqual({ detailsOpen: false, helpOpen: false });
  });

  it("keeps the direct Ollama Polish path behind an explicit development-only seam", () => {
    expect(isDevelopmentPolishAvailable({ explicitlyEnabled: true, isDevelopment: true }))
      .toBe(true);
    expect(isDevelopmentPolishAvailable({ explicitlyEnabled: true, isDevelopment: false }))
      .toBe(false);
    expect(isDevelopmentPolishAvailable({ explicitlyEnabled: false, isDevelopment: true }))
      .toBe(false);
  });

  it("withholds Copy and Save from running or stale Polish drafts", () => {
    expect(isPolishDraftCurrent({
      currentContext: "C:/one.txt\0light",
      draftContext: "C:/one.txt\0light",
      running: false,
      text: "Current draft",
    })).toBe(true);
    expect(isPolishDraftCurrent({
      currentContext: "C:/one.txt\0clean",
      draftContext: "C:/one.txt\0light",
      running: false,
      text: "Stale draft",
    })).toBe(false);
    expect(isPolishDraftCurrent({
      currentContext: "C:/one.txt\0light",
      draftContext: "C:/one.txt\0light",
      running: true,
      text: "Running draft",
    })).toBe(false);
  });
});
