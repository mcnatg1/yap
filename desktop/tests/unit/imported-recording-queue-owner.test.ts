import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const hookSource = readFileSync(
  new URL("../../src/hooks/use-imported-recording-queue.ts", import.meta.url),
  "utf8",
);
const bridgeSource = readFileSync(
  new URL("../../src/recording-queue.ts", import.meta.url),
  "utf8",
);

describe("Rust recording job ownership", () => {
  it("keeps React as a snapshot listener rather than a queue mutator", () => {
    expect(hookSource).toContain("export function useRecordingJobs");
    expect(hookSource).toContain('listen("recording-jobs-changed"');
    expect(hookSource).toContain("recordingJobsSnapshot()");
    expect(hookSource).not.toMatch(/queueRef|nextRecordingId|writeRecordingQueue|allowRecordingPlaybackPath/);
    expect(bridgeSource).not.toMatch(/setItem|createInitialPipelineState|queuedServerMessage/);
  });

  it("subscribes before migration and blocks mutation while migration is unresolved", () => {
    expect(hookSource.indexOf('listen("recording-jobs-changed"'))
      .toBeLessThan(hookSource.indexOf("migrateAndLoad().catch"));
    expect(hookSource).toContain("migrationStateRef.current !== \"ready\"");
    expect(hookSource).toContain("retryMigration: migrateAndLoad");
  });

  it("routes create, remove, retry, and clear through Rust commands", () => {
    expect(hookSource).toContain("createRecordingImports(paths)");
    expect(hookSource).toContain("cancelRecordingJob(id)");
    expect(hookSource).toContain("retryRecordingJob(id)");
    expect(hookSource).toContain("cancelRecordingJob(item.id)");
  });
});
