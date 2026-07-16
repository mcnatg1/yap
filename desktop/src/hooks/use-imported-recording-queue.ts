import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";

import { isRecordingCancellable, type RecordingJobView } from "@/lib/recording-job";
import {
  cancelRecordingJob,
  pickRecordingImports,
  discardLegacyRecordingQueue,
  dismissRecordingJob,
  migrateLegacyRecordingQueue,
  recordingJobsSnapshot,
  retryRecordingJob,
} from "@/recording-queue";
import {
  createRecordingJobsRefreshCoordinator,
  startRecordingJobsLifecycle,
} from "@/recording-jobs-refresh";

type MigrationState = "pending" | "ready" | "failed";

function migrationBlockedError() {
  return new Error("Legacy recording queue migration must finish before jobs can change.");
}

export function useRecordingJobs(onClear: () => void) {
  const [queue, setQueue] = useState<RecordingJobView[]>([]);
  const [migrationState, setMigrationState] = useState<MigrationState>("pending");
  const [migrationError, setMigrationError] = useState<string>();
  const [legacyDiscardAllowed, setLegacyDiscardAllowed] = useState(false);
  const [startupAttempt, setStartupAttempt] = useState(0);
  const migrationStateRef = useRef<MigrationState>("pending");
  const legacyDiscardAllowedRef = useRef(false);
  const refreshCoordinatorRef = useRef<ReturnType<
    typeof createRecordingJobsRefreshCoordinator<RecordingJobView[]>
  > | undefined>(undefined);
  if (!refreshCoordinatorRef.current) {
    refreshCoordinatorRef.current = createRecordingJobsRefreshCoordinator(
      recordingJobsSnapshot,
      setQueue,
    );
  }
  const refresh = refreshCoordinatorRef.current.refresh;
  const onClearRef = useRef(onClear);
  onClearRef.current = onClear;

  const updateMigrationState = useCallback((
    state: MigrationState,
    error?: string,
    allowLegacyDiscard = false,
  ) => {
    migrationStateRef.current = state;
    legacyDiscardAllowedRef.current = allowLegacyDiscard;
    setMigrationState(state);
    setMigrationError(error);
    setLegacyDiscardAllowed(allowLegacyDiscard);
  }, []);

  useEffect(() => {
    updateMigrationState("pending");
    const lifecycle = startRecordingJobsLifecycle({
      failed(error, phase) {
        const legacyMigrationFailed = phase === "migrate";
        const message = legacyMigrationFailed
          ? error.message
          : `Recording jobs could not start: ${error.message}`;
        updateMigrationState("failed", message, legacyMigrationFailed);
        toast.error(legacyMigrationFailed
          ? "The old browser queue was kept. Discard it only when you are ready to re-add recordings through the native picker."
          : "Recording jobs could not start. Retry to continue.");
      },
      migrate: async () => {
        if (isTauri()) await migrateLegacyRecordingQueue();
      },
      ready: () => updateMigrationState("ready"),
      refresh,
      refreshFailed: (error) => {
        toast.error(`Recording jobs could not be refreshed: ${error.message}`);
      },
      subscribe: (handler) => isTauri()
        ? listen("recording-jobs-changed", handler)
        : Promise.resolve(() => {}),
    });
    return lifecycle.dispose;
  }, [refresh, startupAttempt, updateMigrationState]);

  const ensureMigrationReady = useCallback(() => {
    if (migrationStateRef.current !== "ready") throw migrationBlockedError();
  }, []);

  const discardLegacyQueue = useCallback(() => {
    if (!legacyDiscardAllowedRef.current) {
      throw new Error("Legacy queue discard is unavailable for this startup failure.");
    }
    discardLegacyRecordingQueue();
    updateMigrationState("pending");
    setStartupAttempt((attempt) => attempt + 1);
  }, [updateMigrationState]);

  const addRecordings = useCallback(async () => {
    ensureMigrationReady();
    const created = await pickRecordingImports();
    await refresh();
    return created[created.length - 1]?.id;
  }, [ensureMigrationReady, refresh]);

  const removeItem = useCallback(async (id: string) => {
    ensureMigrationReady();
    const item = queue.find((entry) => entry.id === id);
    if (!item) return;
    if (item.status === "failed") {
      await dismissRecordingJob(id);
    } else if (isRecordingCancellable(item.status)) {
      await cancelRecordingJob(id);
    } else {
      return;
    }
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
      if (item.status === "failed") {
        await dismissRecordingJob(item.id);
      } else if (isRecordingCancellable(item.status)) {
        await cancelRecordingJob(item.id);
      }
    }
    await refresh();
    onClearRef.current();
  }, [ensureMigrationReady, queue, refresh]);

  return {
    addRecordings,
    clearQueue,
    discardLegacyQueue,
    legacyDiscardAllowed,
    migrationError,
    migrationState,
    queue,
    refresh,
    removeItem,
    retryItem,
    retryMigration: () => setStartupAttempt((attempt) => attempt + 1),
  };
}
