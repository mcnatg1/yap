import { afterEach, describe, expect, it, vi } from "vitest";

import { createNativeSurfaceSync } from "@/components/live/native-surface-sync";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

async function tick() {
  await Promise.resolve();
}

describe("native overlay surface sync", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("collapses rapid changes behind the in-flight native resize", async () => {
    const first = deferred();
    const calls: string[] = [];
    const sync = createNativeSurfaceSync(async ({ surface }) => {
      calls.push(surface);
      if (calls.length === 1) await first.promise;
    });

    sync({ surface: "collapsed" });
    sync({ surface: "expanded" });
    sync({ surface: "recording" });
    await tick();

    expect(calls).toEqual(["collapsed"]);

    first.resolve();
    await tick();
    await tick();

    expect(calls).toEqual(["collapsed", "recording"]);
  });

  it("keeps draining after a native resize failure", async () => {
    const calls: string[] = [];
    const sync = createNativeSurfaceSync(async ({ surface }) => {
      calls.push(surface);
      if (surface === "collapsed") throw new Error("resize failed");
    });

    sync({ surface: "collapsed" });
    sync({ surface: "expanded" });
    await tick();
    await tick();

    expect(calls).toEqual(["collapsed", "expanded"]);
  });

  it("retries the latest failed native resize", async () => {
    vi.useFakeTimers();
    const calls: string[] = [];
    const sync = createNativeSurfaceSync(
      async ({ surface }) => {
        calls.push(surface);
        if (calls.length === 1) throw new Error("resize failed");
      },
      { maxRetries: 1, retryDelayMs: 25 },
    );

    sync({ surface: "collapsed" });
    await tick();
    expect(calls).toEqual(["collapsed"]);

    await vi.advanceTimersByTimeAsync(25);
    await tick();

    expect(calls).toEqual(["collapsed", "collapsed"]);
  });
});
