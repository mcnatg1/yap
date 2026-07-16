import { describe, expect, it } from "vitest";

import {
  pruneMissingHiddenTranscriptHistory,
  readHiddenTranscriptHistory,
  readTranscriptHistory,
  readVisibleTranscriptHistory,
  writeTranscriptHistory,
  type OwnedLiveTranscriptPathResolution,
} from "@/history-storage";

function missingResolution(requestedPath: string): OwnedLiveTranscriptPathResolution {
  return { canonicalPath: null, missing: true, requestedPath };
}

describe("hidden transcript maintenance", () => {
  it("prunes only Rust-confirmed hidden outputs", async () => {
    const values = new Map([["yap.hiddenTranscriptHistory.v1", JSON.stringify(["existing", "missing", "external"])]]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    const next = await pruneMissingHiddenTranscriptHistory(async () => [
      missingResolution("missing"),
      missingResolution("not-requested"),
    ], storage);

    expect(next).toEqual(["existing", "external"]);
    expect(readHiddenTranscriptHistory(storage)).toEqual(["existing", "external"]);
  });

  it("preserves tombstones added while Rust authorization is pending", async () => {
    let raw = JSON.stringify(["missing", "existing"]);
    let resolveAuthorization!: (paths: OwnedLiveTranscriptPathResolution[]) => void;
    const storage = {
      getItem: () => raw,
      setItem: (_key: string, value: string) => { raw = value; },
    };
    const authorization = new Promise<OwnedLiveTranscriptPathResolution[]>((resolve) => {
      resolveAuthorization = resolve;
    });

    const pruning = pruneMissingHiddenTranscriptHistory(() => authorization, storage);
    await Promise.resolve();
    raw = JSON.stringify(["newly-hidden", "missing", "existing"]);
    resolveAuthorization([missingResolution("missing")]);
    await pruning;

    expect(readHiddenTranscriptHistory(storage)).toEqual(["newly-hidden", "existing"]);
  });

  it("migrates an existing alias tombstone to Rust's canonical path", async () => {
    const alias = "C:\\Alias\\live-1.txt";
    const canonical = "D:\\Real\\live-1.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([alias])],
      ["yap.transcriptHistory.v1", JSON.stringify([
        {
          createdAt: "2026-01-02T00:00:00.000Z",
          name: "live-1",
          outputPath: canonical,
          sourcePath: canonical,
        },
        {
          createdAt: "2026-01-01T00:00:00.000Z",
          name: "live-1-alias",
          outputPath: alias,
          sourcePath: alias,
        },
      ])],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    await pruneMissingHiddenTranscriptHistory(async () => [{
      canonicalPath: canonical,
      missing: false,
      requestedPath: alias,
    }], storage);

    expect(readHiddenTranscriptHistory(storage)).toEqual([canonical]);
    expect(readTranscriptHistory(storage).map((entry) => entry.outputPath)).toEqual([canonical]);
    expect(readVisibleTranscriptHistory(storage)).toEqual([]);
  });

  it("removes canonical history when a missing alias resolves after deletion", async () => {
    const alias = "C:\\Alias\\live-2.txt";
    const canonical = "D:\\Real\\live-2.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([alias])],
      ["yap.transcriptHistory.v1", JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-2",
        outputPath: canonical,
        sourcePath: canonical,
      }])],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };

    await pruneMissingHiddenTranscriptHistory(async () => [{
      canonicalPath: canonical,
      missing: true,
      requestedPath: alias,
    }], storage);

    expect(readTranscriptHistory(storage)).toEqual([]);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
  });

  it("protects a canonical tombstone before removing stale alias metadata", async () => {
    const alias = "C:\\Alias\\live-3.txt";
    const canonical = "D:\\Real\\live-3.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([alias])],
      ["yap.transcriptHistory.v1", JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-3-alias",
        outputPath: alias,
        sourcePath: alias,
      }])],
    ]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        if (key === "yap.transcriptHistory.v1") throw new Error("quota full");
        values.set(key, value);
      },
    };

    await expect(pruneMissingHiddenTranscriptHistory(async () => [{
      canonicalPath: canonical,
      missing: false,
      requestedPath: alias,
    }], storage)).rejects.toThrow("quota full");

    expect(readHiddenTranscriptHistory(storage)).toEqual([alias, canonical]);
  });

  it("removes stale history metadata before pruning its tombstone", async () => {
    const missing = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-101.txt";
    const values = new Map([
      ["yap.hiddenTranscriptHistory.v1", JSON.stringify([missing])],
      ["yap.transcriptHistory.v1", JSON.stringify([{
        createdAt: "2026-01-01T00:00:00.000Z",
        name: "live-101",
        outputPath: missing,
        sourcePath: missing,
      }])],
    ]);
    const writes: string[] = [];
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => {
        writes.push(key);
        values.set(key, value);
      },
    };

    await pruneMissingHiddenTranscriptHistory(async () => [missingResolution(missing)], storage);

    expect(readTranscriptHistory(storage)).toEqual([]);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
    expect(writes).toEqual([
      "yap.transcriptHistory.v1",
      "yap.hiddenTranscriptHistory.v1",
    ]);
  });

  it("keeps the tombstone when stale history cleanup cannot be saved", async () => {
    const missing = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-102.txt";
    const history = JSON.stringify([{
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-102",
      outputPath: missing,
      sourcePath: missing,
    }]);
    const hidden = JSON.stringify([missing]);
    const storage = {
      getItem: (key: string) => key === "yap.transcriptHistory.v1" ? history : hidden,
      setItem: (key: string) => {
        if (key === "yap.transcriptHistory.v1") throw new Error("quota full");
        throw new Error("tombstone must not be written");
      },
    };

    await expect(pruneMissingHiddenTranscriptHistory(async () => [missingResolution(missing)], storage))
      .rejects.toThrow("quota full");
    expect(readHiddenTranscriptHistory(storage)).toEqual([missing]);
  });

  it("removes a stale rescan row that resolves while pruning is pending", async () => {
    const missing = "C:\\Users\\me\\AppData\\Local\\Yap\\live-recordings\\live-103.txt";
    const values = new Map([["yap.hiddenTranscriptHistory.v1", JSON.stringify([missing])]]);
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    };
    let resolveAuthorization!: (paths: OwnedLiveTranscriptPathResolution[]) => void;
    const authorization = new Promise<OwnedLiveTranscriptPathResolution[]>((resolve) => {
      resolveAuthorization = resolve;
    });

    const pruning = pruneMissingHiddenTranscriptHistory(() => authorization, storage);
    await Promise.resolve();
    writeTranscriptHistory([{
      createdAt: "2026-01-01T00:00:00.000Z",
      name: "live-103",
      outputPath: missing,
      sourcePath: missing,
    }], storage);
    resolveAuthorization([missingResolution(missing)]);
    await pruning;

    expect(readTranscriptHistory(storage)).toEqual([]);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
  });

  it("does not write tombstones when an authorization batch fails", async () => {
    const original = JSON.stringify(Array.from({ length: 201 }, (_, index) => `live-${index}.txt`));
    let raw = original;
    let writes = 0;
    const storage = {
      getItem: () => raw,
      setItem: (_key: string, value: string) => {
        writes += 1;
        raw = value;
      },
    };
    let calls = 0;

    await expect(pruneMissingHiddenTranscriptHistory(async () => {
      calls += 1;
      if (calls === 2) throw new Error("native unavailable");
      return [missingResolution("live-0.txt")];
    }, storage)).rejects.toThrow("native unavailable");

    expect(calls).toBe(2);
    expect(writes).toBe(0);
    expect(raw).toBe(original);
  });

  it("batches tombstone authorization and commits once", async () => {
    const paths = Array.from({ length: 201 }, (_, index) => `live-${index}.txt`);
    let raw = JSON.stringify(paths);
    let writes = 0;
    const batchSizes: number[] = [];
    const storage = {
      getItem: () => raw,
      setItem: (_key: string, value: string) => {
        writes += 1;
        raw = value;
      },
    };

    await pruneMissingHiddenTranscriptHistory(async (batch) => {
      batchSizes.push(batch.length);
      return batch.map(missingResolution);
    }, storage);

    expect(batchSizes).toEqual([200, 1]);
    expect(writes).toBe(1);
    expect(readHiddenTranscriptHistory(storage)).toEqual([]);
  });
});
