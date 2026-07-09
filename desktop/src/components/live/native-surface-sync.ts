import type { OverlaySurface } from "@/components/live/live-overlay-state";

type NativeSurfaceRequest = {
  errorMessage?: string;
  surface: OverlaySurface;
};

type NativeSurfaceInvoke = (request: NativeSurfaceRequest) => Promise<void>;

export function createNativeSurfaceSync(invokeNative: NativeSurfaceInvoke) {
  let latest: NativeSurfaceRequest | undefined;
  let running = false;

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
          // Native overlay resize is best-effort; React remains the visual source of truth.
        }
      }
    } finally {
      running = false;
      if (latest) void drain();
    }
  }

  return (request: NativeSurfaceRequest) => {
    latest = request;
    void drain();
  };
}
