import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Buffer } from "node:buffer";
import { transformSync } from "esbuild";

const source = readFileSync(new URL("../src/history.ts", import.meta.url), "utf8");
const { code } = transformSync(source, { format: "esm", loader: "ts" });
const history = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);

const store = new Map();
const storage = {
  getItem: (key) => store.get(key) ?? null,
  setItem: (key, value) => store.set(key, value),
};

const older = {
  name: "older.wav",
  sourcePath: "C:\\audio\\older.wav",
  outputPath: "C:\\audio\\older.txt",
  createdAt: "2026-01-01T00:00:00.000Z",
};
const newer = {
  name: "newer.wav",
  sourcePath: "C:\\audio\\newer.wav",
  outputPath: "C:\\audio\\newer.txt",
  createdAt: "2026-01-02T00:00:00.000Z",
};

history.writeTranscriptHistory([older, newer], storage);
assert.deepEqual(history.readTranscriptHistory(storage).map((entry) => entry.name), ["newer.wav", "older.wav"]);

const updatedOlder = { ...older, name: "renamed.wav", createdAt: "2026-01-03T00:00:00.000Z" };
const recorded = history.recordTranscriptHistory(history.readTranscriptHistory(storage), updatedOlder);
assert.equal(recorded.length, 2);
assert.deepEqual(recorded.map((entry) => entry.name), ["renamed.wav", "newer.wav"]);

const removed = history.removeTranscriptHistory(recorded, newer.outputPath);
assert.deepEqual(removed.map((entry) => entry.outputPath), [older.outputPath]);

store.set(history.transcriptHistoryKey, "{broken");
assert.deepEqual(history.readTranscriptHistory(storage), []);

console.log("history check passed");
