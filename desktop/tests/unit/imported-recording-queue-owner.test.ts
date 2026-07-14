import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";

import {
  createRecordingJobsRefreshCoordinator,
  startRecordingJobsLifecycle,
} from "@/recording-jobs-refresh";

const hookSource = readFileSync(
  new URL("../../src/hooks/use-imported-recording-queue.ts", import.meta.url),
  "utf8",
);
const bridgeSource = readFileSync(
  new URL("../../src/recording-queue.ts", import.meta.url),
  "utf8",
);
const refreshSource = readFileSync(
  new URL("../../src/recording-jobs-refresh.ts", import.meta.url),
  "utf8",
);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

describe("Rust recording job ownership", () => {
  it("keeps React as a snapshot listener rather than a queue mutator", () => {
    expect(hookSource).toContain("export function useRecordingJobs");
    expect(hookSource).toContain('listen("recording-jobs-changed"');
    expect(hookSource).toContain("recordingJobsSnapshot,");
    expect(hookSource).not.toMatch(/queueRef|nextRecordingId|writeRecordingQueue|allowRecordingPlaybackPath/);
    expect(bridgeSource).not.toMatch(/setItem|createInitialPipelineState|queuedServerMessage/);
  });

  it("subscribes before migration and blocks mutation while migration is unresolved", () => {
    expect(refreshSource.indexOf("await subscribe"))
      .toBeLessThan(refreshSource.indexOf("await migrate"));
    expect(hookSource).toContain("migrationStateRef.current !== \"ready\"");
    expect(hookSource).toContain("retryMigration: () => setStartupAttempt");
  });

  it("allows legacy discard only for a migration-phase startup failure", () => {
    expect(hookSource).toContain('phase === "migrate"');
    expect(hookSource).toContain("legacyDiscardAllowedRef.current");
    expect(hookSource).toContain("discardLegacyRecordingQueue()");
    expect(hookSource).toContain("discardLegacyQueue");
    const discardOwner = hookSource.slice(
      hookSource.indexOf("const discardLegacyQueue"),
      hookSource.indexOf("const addRecordings"),
    );
    expect(discardOwner).toContain("setStartupAttempt");
  });

  it("routes native picker create, remove, retry, and clear through Rust commands", () => {
    expect(hookSource).toContain("pickRecordingImports()");
    expect(bridgeSource).toContain('invoke<RecordingJobView[]>("recording_jobs_pick_imports")');
    expect(bridgeSource).not.toMatch(/recording_jobs_create_imports|recording_jobs_import_legacy/);
    expect(hookSource).toContain("cancelRecordingJob(id)");
    expect(hookSource).toContain("dismissRecordingJob(id)");
    expect(hookSource).toContain("retryRecordingJob(id)");
    expect(hookSource).toContain("cancelRecordingJob(item.id)");
    expect(hookSource).toContain("dismissRecordingJob(item.id)");
  });

  it("runs a trailing snapshot when an event arrives during an in-flight refresh", async () => {
    const stale = deferred<string[]>();
    const stable = deferred<string[]>();
    const load = vi.fn()
      .mockImplementationOnce(() => stale.promise)
      .mockImplementationOnce(() => stable.promise);
    const applied: string[][] = [];
    const coordinator = createRecordingJobsRefreshCoordinator(load, (snapshot) => {
      applied.push(snapshot);
    });

    const initialRefresh = coordinator.refresh();
    const eventRefresh = coordinator.refresh();
    stale.resolve(["before-commit"]);
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
    stable.resolve(["after-commit"]);

    await expect(initialRefresh).resolves.toEqual(["after-commit"]);
    await expect(eventRefresh).resolves.toEqual(["after-commit"]);
    expect(applied).toEqual([["before-commit"], ["after-commit"]]);
  });

  it("publishes ready only after subscription, migration, and a stable snapshot", async () => {
    const stale = deferred<string[]>();
    const stable = deferred<string[]>();
    const load = vi.fn()
      .mockImplementationOnce(() => stale.promise)
      .mockImplementationOnce(() => stable.promise);
    const coordinator = createRecordingJobsRefreshCoordinator(load, vi.fn());
    const ready = vi.fn();
    let publishEvent!: () => void;
    const lifecycle = startRecordingJobsLifecycle({
      failed: vi.fn(),
      migrate: vi.fn().mockResolvedValue(undefined),
      ready,
      refresh: coordinator.refresh,
      refreshFailed: vi.fn(),
      subscribe: vi.fn(async (handler) => {
        publishEvent = handler;
        return vi.fn();
      }),
    });

    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
    publishEvent();
    stale.resolve(["before-event"]);
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
    expect(ready).not.toHaveBeenCalled();
    stable.resolve(["after-event"]);
    await lifecycle.settled;

    expect(ready).toHaveBeenCalledTimes(1);
  });

  it("fails startup without migrating when listener registration rejects", async () => {
    const migrate = vi.fn();
    const ready = vi.fn();
    const failed = vi.fn();
    const lifecycle = startRecordingJobsLifecycle({
      failed,
      migrate,
      ready,
      refresh: vi.fn(),
      refreshFailed: vi.fn(),
      subscribe: vi.fn().mockRejectedValue(new Error("listener unavailable")),
    });

    await lifecycle.settled;

    expect(failed).toHaveBeenCalledWith(
      expect.objectContaining({ message: "listener unavailable" }),
      "subscribe",
    );
    expect(migrate).not.toHaveBeenCalled();
    expect(ready).not.toHaveBeenCalled();
  });

  it("attributes migration and snapshot startup failures to their lifecycle phases", async () => {
    const migrationFailed = vi.fn();
    const migrationLifecycle = startRecordingJobsLifecycle({
      failed: migrationFailed,
      migrate: vi.fn().mockRejectedValue(new Error("legacy JSON is malformed")),
      ready: vi.fn(),
      refresh: vi.fn(),
      refreshFailed: vi.fn(),
      subscribe: vi.fn().mockResolvedValue(vi.fn()),
    });
    await migrationLifecycle.settled;
    expect(migrationFailed).toHaveBeenCalledWith(
      expect.objectContaining({ message: "legacy JSON is malformed" }),
      "migrate",
    );

    const snapshotFailed = vi.fn();
    const snapshotLifecycle = startRecordingJobsLifecycle({
      failed: snapshotFailed,
      migrate: vi.fn().mockResolvedValue(undefined),
      ready: vi.fn(),
      refresh: vi.fn().mockRejectedValue(new Error("snapshot unavailable")),
      refreshFailed: vi.fn(),
      subscribe: vi.fn().mockResolvedValue(vi.fn()),
    });
    await snapshotLifecycle.settled;
    expect(snapshotFailed).toHaveBeenCalledWith(
      expect.objectContaining({ message: "snapshot unavailable" }),
      "refresh",
    );
  });

  it("unlistens a listener that resolves after lifecycle disposal", async () => {
    const listener = deferred<() => void>();
    const unlisten = vi.fn();
    const migrate = vi.fn();
    const lifecycle = startRecordingJobsLifecycle({
      failed: vi.fn(),
      migrate,
      ready: vi.fn(),
      refresh: vi.fn(),
      refreshFailed: vi.fn(),
      subscribe: vi.fn(() => listener.promise),
    });

    lifecycle.dispose();
    listener.resolve(unlisten);
    await lifecycle.settled;

    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(migrate).not.toHaveBeenCalled();
  });
});
