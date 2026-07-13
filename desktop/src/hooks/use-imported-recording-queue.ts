import { isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { isRecordingCancellable, type RecordingJobView } from "@/lib/app-types";
import {
  cancelRecordingJob,
  createRecordingImports,
  migrateLegacyRecordingQueue,
  recordingJobsSnapshot,
  retryRecordingJob,
} from "@/recording-queue";

type MigrationState = "pending" | "ready" | "failed";

function migrationBlockedError() {
  return new Error("Legacy recording queue migration must finish before jobs can change.");
}

export function useRecordingJobs(onClear: () => void) {
  const [queue, setQueue] = useState<RecordingJobView[]>([]);
  const [migrationState, setMigrationState] = useState<MigrationState>("pending");
  const [migrationError, setMigrationError] = useState<string>();
  const migrationStateRef = useRef<MigrationState>("pending");
  const refreshPromiseRef = useRef<Promise<RecordingJobView[]> | undefined>(undefined);
  const onClearRef = useRef(onClear);
  onClearRef.current = onClear;

  const updateMigrationState = useCallback((state: MigrationState, error?: string) => {
    migrationStateRef.current = state;
    setMigrationState(state);
    setMigrationError(error);
  }, []);

  const refresh = useCallback(async () => {
    if (refreshPromiseRef.current) return refreshPromiseRef.current;
    const pending = recordingJobsSnapshot().then((snapshot) => {
      setQueue(snapshot);
      return snapshot;
    }).finally(() => {
      if (refreshPromiseRef.current === pending) refreshPromiseRef.current = undefined;
    });
    refreshPromiseRef.current = pending;
    return pending;
  }, []);

  const migrateAndLoad = useCallback(async () => {
    updateMigrationState("pending");
    try {
      if (isTauri()) await migrateLegacyRecordingQueue();
      updateMigrationState("ready");
      return await refresh();
    } catch (error) {
      const message = `Queued recording migration needs attention: ${String(error)}`;
      updateMigrationState("failed", message);
      throw error;
    }
  }, [refresh, updateMigrationState]);

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | undefined;
    void (async () => {
      if (isTauri()) {
        try {
          unlisten = await listen("recording-jobs-changed", () => {
            if (active && migrationStateRef.current === "ready") {
              void refresh().catch((error) => {
                toast.error(`Recording jobs could not be refreshed: ${String(error)}`);
              });
            }
          });
        } catch (error) {
          console.warn("Recording job updates could not be subscribed.", error);
        }
      }
      if (!active) {
        unlisten?.();
        return;
      }
      await migrateAndLoad().catch(() => {
        if (active) toast.error("Queued recordings could not be migrated. Retry to continue.");
      });
    })();

    return () => {
      active = false;
      unlisten?.();
    };
  }, [migrateAndLoad, refresh]);

  const ensureMigrationReady = useCallback(() => {
    if (migrationStateRef.current !== "ready") throw migrationBlockedError();
  }, []);

  const addPaths = useCallback(async (paths: string[]) => {
    ensureMigrationReady();
    const created = await createRecordingImports(paths);
    await refresh();
    return created[created.length - 1]?.id;
  }, [ensureMigrationReady, refresh]);

  const removeItem = useCallback(async (id: string) => {
    ensureMigrationReady();
    const item = queue.find((entry) => entry.id === id);
    if (!item || !isRecordingCancellable(item.status)) return;
    await cancelRecordingJob(id);
    await refresh();
  }, [ensureMigrationReady, queue, refresh]);

  const retryItem = useCallback(async (id: string) => {
    ensureMigrationReady();
    await retryRecordingJob(id);
    await refresh();
  }, [ensureMigrationReady, refresh]);

  const clearQueue = useCallback(async () => {
    ensureMigrationReady();
    for (const item of queue) {
      if (isRecordingCancellable(item.status)) {
        await cancelRecordingJob(item.id);
      }
    }
    await refresh();
    onClearRef.current();
  }, [ensureMigrationReady, queue, refresh]);

  return {
    addPaths,
    clearQueue,
    migrationError,
    migrationState,
    queue,
    refresh,
    removeItem,
    retryItem,
    retryMigration: migrateAndLoad,
  };
}
