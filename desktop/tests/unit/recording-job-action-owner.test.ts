import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const appSource = readFileSync(
  new URL("../../src/App.tsx", import.meta.url),
  "utf8",
);

describe("recording job action ownership", () => {
  it("reports rejected queue actions instead of discarding their promises", () => {
    expect(appSource).toContain('import { fireAndReport } from "@/lib/fire-and-report"');
    expect(appSource).not.toContain("onClear={clearQueue}");
    expect(appSource).not.toContain("onRemove={removeItem}");
    expect(appSource).not.toContain("onDiscardLegacyQueue={discardLegacyQueue}");
    expect(appSource).not.toContain("onRetry={(id) => void retryItem(id)}");
    expect(appSource).toContain("Could not clear queue");
    expect(appSource).toContain("Could not remove recording");
    expect(appSource).toContain("Could not discard old queue");
    expect(appSource).toContain("Could not retry recording");
  });
});
