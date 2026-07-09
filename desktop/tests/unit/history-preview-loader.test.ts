import { describe, expect, it } from "vitest";

import { createPreviewTextLoader } from "@/lib/history-preview-loader";

describe("history preview loader", () => {
  it("dedupes concurrent reads by output path", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "C:\\recordings\\live-1.txt" };
    let reads = 0;
    const loaded: Record<string, string> = {};
    const readText = async () => {
      reads += 1;
      return "hello";
    };

    const [first, second] = await Promise.all([
      loader.load(entry, {}, readText, (path, text) => {
        loaded[path] = text;
      }),
      loader.load(entry, {}, readText, (path, text) => {
        loaded[path] = text;
      }),
    ]);

    expect(first).toBe("hello");
    expect(second).toBe("hello");
    expect(reads).toBe(1);
    expect(loaded[entry.outputPath]).toBe("hello");
  });

  it("returns cached empty previews without reading again", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "C:\\recordings\\live-empty.txt" };
    let reads = 0;

    const text = await loader.load(entry, { [entry.outputPath]: "" }, async () => {
      reads += 1;
      return "unexpected";
    }, () => undefined);

    expect(text).toBe("");
    expect(reads).toBe(0);
  });

  it("rejects failed reads so the row can show its fallback state", async () => {
    const loader = createPreviewTextLoader();
    const entry = { outputPath: "C:\\recordings\\missing.txt" };

    await expect(loader.load(entry, {}, async () => {
      throw new Error("missing");
    }, () => undefined)).rejects.toThrow("missing");
  });
});
