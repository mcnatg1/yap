import { writeFileSync } from "node:fs";
import path from "node:path";
import { afterEach } from "vitest";

import {
  createWdioRunIsolation,
  removePrivateRunRootWhenReleased,
} from "../wdio/task-8b-isolation.js";

function writeCanonicalBundle(recordingRoot, name = "live-s-18c13f2a28c8be80-d018-2") {
  const suffixes = [
    ".capture.json",
    ".commit.json",
    ".transcript.r1.json",
    ".txt",
    ".wav",
  ];
  for (const suffix of suffixes) {
    writeFileSync(
      path.join(recordingRoot, `${name}${suffix}`),
      suffix === ".wav" ? Buffer.alloc(64) : "{}\n",
    );
  }
  return {
    captureCommitPath: path.join(recordingRoot, `${name}.commit.json`),
    createdAtMs: 2_000,
    name,
    outputPath: path.join(recordingRoot, `${name}.txt`),
    sessionId: name.slice("live-".length),
    sourcePath: path.join(recordingRoot, `${name}.wav`),
  };
}

export function installTask8bPrivateIsolationFixture() {
  const privateIsolations = [];
  let fixtureSequence = 0;

  function privateIsolation(label, env = {}) {
    fixtureSequence += 1;
    const safeLabel = label
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "");
    const isolation = createWdioRunIsolation(env, {
      token: `unit-${safeLabel}-${process.pid}-${Date.now()}-${fixtureSequence}`,
    });
    privateIsolations.push(isolation);
    return isolation;
  }

  afterEach(async () => {
    const failures = [];
    while (privateIsolations.length) {
      const isolation = privateIsolations.pop();
      try {
        await removePrivateRunRootWhenReleased(isolation);
      } catch (error) {
        failures.push(error);
      }
    }
    if (failures.length > 0) {
      throw new AggregateError(failures, "Task 8b unit fixture cleanup failed");
    }
  });

  return { privateIsolation, writeCanonicalBundle };
}
