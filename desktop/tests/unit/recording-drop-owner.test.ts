import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const hookSource = readFileSync(
  new URL("../../src/hooks/use-recording-drop.ts", import.meta.url),
  "utf8",
);

describe("recording drop ownership", () => {
  it("keeps dropped paths native while preserving visual and error feedback", () => {
    expect(hookSource).not.toMatch(/event\.payload\.paths|\{ paths \}/);
    expect(hookSource).toContain('listen<string>("recording-jobs-import-error"');
    expect(hookSource).toMatch(/toast\.error\(`Could not add recordings:/);
    expect(hookSource).toContain('if (!isTauri()) toast.info("Preview only")');
  });
});
