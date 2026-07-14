import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const hostSource = readFileSync(
  new URL("../../src/components/live/live-overlay-host.tsx", import.meta.url),
  "utf8",
);

describe("live overlay native subscription", () => {
  it("installs invalidation before taking the first native snapshot", () => {
    const setupStart = hostSource.indexOf("let cancelled = false;");
    const subscribe = hostSource.indexOf(
      "void listenLiveOverlaySession(refreshFromNative)",
      setupStart,
    );
    const listenerReady = hostSource.indexOf("unlisten = stop;", subscribe);
    const initialRefresh = hostSource.indexOf("refreshFromNative();", listenerReady);

    expect(setupStart).toBeGreaterThanOrEqual(0);
    expect(subscribe).toBeGreaterThan(setupStart);
    expect(hostSource.slice(setupStart, subscribe)).not.toContain("refreshFromNative();");
    expect(listenerReady).toBeGreaterThan(subscribe);
    expect(initialRefresh).toBeGreaterThan(listenerReady);
  });
});
