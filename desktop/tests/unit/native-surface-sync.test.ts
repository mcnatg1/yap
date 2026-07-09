import { describe, expect, it } from "vitest";

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
  it("collapses rapid changes behind the in-flight native resize", async () => {
    const first = deferred();
    const calls: string[] = [];
    const sync = createNativeSurfaceSync(async ({ surface }) => {
      calls.push(surface);
      if (calls.length === 1) await first.promise;
    });

    sync({ surface: "sensor" });
    sync({ surface: "peek" });
    sync({ surface: "recording" });
    await tick();

    expect(calls).toEqual(["sensor"]);

    first.resolve();
    await tick();
    await tick();

    expect(calls).toEqual(["sensor", "recording"]);
  });

  it("keeps draining after a native resize failure", async () => {
    const calls: string[] = [];
    const sync = createNativeSurfaceSync(async ({ surface }) => {
      calls.push(surface);
      if (surface === "sensor") throw new Error("resize failed");
    });

    sync({ surface: "sensor" });
    sync({ surface: "peek" });
    await tick();
    await tick();

    expect(calls).toEqual(["sensor", "peek"]);
  });
});
