import type { OverlaySurface } from "@/components/live/live-overlay-state";

type NativeSurfaceRequest = {
  errorMessage?: string;
  surface: OverlaySurface;
};

type PendingSurfaceRequest = NativeSurfaceRequest & {
  attempts: number;
};

type NativeSurfaceInvoke = (request: NativeSurfaceRequest) => Promise<void>;

export function createNativeSurfaceSync(
  invokeNative: NativeSurfaceInvoke,
  options: { maxRetries?: number; retryDelayMs?: number } = {},
) {
  const maxRetries = options.maxRetries ?? 2;
  const retryDelayMs = options.retryDelayMs ?? 80;
  let latest: PendingSurfaceRequest | undefined;
  let running = false;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;

  async function drain() {
    if (running) return;
    running = true;
    try {
      while (latest) {
        const request = latest;
        latest = undefined;
        try {
          await invokeNative(request);
        } catch {
          if (!latest && request.attempts < maxRetries) {
            retryTimer = setTimeout(() => {
              retryTimer = undefined;
              latest = { ...request, attempts: request.attempts + 1 };
              void drain();
            }, retryDelayMs);
          }
        }
      }
    } finally {
      running = false;
      if (latest) void drain();
    }
  }

  return (request: NativeSurfaceRequest) => {
    if (retryTimer !== undefined) {
      clearTimeout(retryTimer);
      retryTimer = undefined;
    }
    latest = { ...request, attempts: 0 };
    void drain();
  };
}
