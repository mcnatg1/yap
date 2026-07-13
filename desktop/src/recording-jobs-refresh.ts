export type RecordingJobsRefreshCoordinator<T> = {
  refresh: () => Promise<T>;
};

export function createRecordingJobsRefreshCoordinator<T>(
  load: () => Promise<T>,
  apply: (snapshot: T) => void,
): RecordingJobsRefreshCoordinator<T> {
  let dirty = false;
  let running: Promise<T> | undefined;

  async function drain() {
    let latest!: T;
    do {
      dirty = false;
      latest = await load();
      apply(latest);
    } while (dirty);
    return latest;
  }

  return {
    refresh() {
      dirty = true;
      if (!running) {
        const pending = drain();
        const tracked = pending.finally(() => {
          if (running === tracked) running = undefined;
        });
        running = tracked;
      }
      return running;
    },
  };
}

type RecordingJobsLifecycleOptions = {
  failed: (error: Error) => void;
  migrate: () => Promise<unknown>;
  ready: () => void;
  refresh: () => Promise<unknown>;
  refreshFailed: (error: Error) => void;
  subscribe: (handler: () => void) => Promise<() => void>;
};

function asError(error: unknown) {
  return error instanceof Error ? error : new Error(String(error));
}

export function startRecordingJobsLifecycle({
  failed,
  migrate,
  ready,
  refresh,
  refreshFailed,
  subscribe,
}: RecordingJobsLifecycleOptions) {
  let active = true;
  let unlisten: (() => void) | undefined;

  const settled = (async () => {
    try {
      const subscribed = await subscribe(() => {
        if (!active) return;
        void refresh().catch((error) => {
          if (active) refreshFailed(asError(error));
        });
      });
      if (!active) {
        subscribed();
        return;
      }
      unlisten = subscribed;
      await migrate();
      if (!active) return;
      await refresh();
      if (active) ready();
    } catch (error) {
      if (active) failed(asError(error));
    }
  })();

  return {
    dispose() {
      active = false;
      unlisten?.();
      unlisten = undefined;
    },
    settled,
  };
}
