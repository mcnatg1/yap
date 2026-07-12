import { describe, expect, it } from "vitest";

import { historyRenderWindowSize, renderHistoryWindow } from "@/lib/history-render-window";

describe("history render window", () => {
  it("renders the first history window and reports hidden rows", () => {
    const entries = Array.from({ length: historyRenderWindowSize + 5 }, (_, index) => index);

    const window = renderHistoryWindow(entries);

    expect(window.visibleEntries).toHaveLength(historyRenderWindowSize);
    expect(window.hiddenCount).toBe(5);
    expect(window.nextLimit).toBe(entries.length);
  });

  it("clamps invalid limits without expanding the list", () => {
    const window = renderHistoryWindow([1, 2, 3], -10);

    expect(window.visibleEntries).toEqual([]);
    expect(window.hiddenCount).toBe(3);
    expect(window.nextLimit).toBe(3);
  });

  it("advances by another fixed window", () => {
    const entries = Array.from({ length: historyRenderWindowSize * 3 }, (_, index) => index);

    const window = renderHistoryWindow(entries, historyRenderWindowSize + 10);

    expect(window.visibleEntries).toHaveLength(historyRenderWindowSize + 10);
    expect(window.hiddenCount).toBe(historyRenderWindowSize * 2 - 10);
    expect(window.nextLimit).toBe(historyRenderWindowSize * 2 + 10);
  });
});
