import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const hookSource = readFileSync(
  new URL("../../src/hooks/use-recording-drop.ts", import.meta.url),
  "utf8",
);

describe("recording drop ownership", () => {
  it("reports a rejected native recording drop without changing browser preview behavior", () => {
    expect(hookSource).toContain("fireAndReport(");
    expect(hookSource).toContain("const { paths } = event.payload");
    expect(hookSource).toContain("onDropPathsRef.current(paths)");
    expect(hookSource).toMatch(/toast\.error\([^)]*Could not add recordings/);
    expect(hookSource).toContain('if (!isTauri()) toast.info("Preview only")');
  });
});
