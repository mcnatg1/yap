export async function registerTask8bLifecycleListeners(_tauri, options = {}) {
  const event = globalThis.__TAURI__?.event;
  if (!event?.listen) throw new Error("Tauri event API is unavailable in the current WebView.");
  const target = options.target ?? "overlay";
  if (target !== "main" && target !== "overlay") {
    throw new Error("Lifecycle listener target must be main or overlay.");
  }

  const state = {
    levels: [],
    saved: [],
    sessions: [],
    unlisteners: [],
  };
  globalThis.__yapTask8bLifecycle = state;
  state.cleanup = async () => {
    const pending = [...state.unlisteners];
    let cleaned = 0;
    const failures = [];
    for (const unlisten of pending) {
      try {
        await unlisten();
        const index = state.unlisteners.indexOf(unlisten);
        if (index >= 0) {
          state.unlisteners.splice(index, 1);
          cleaned += 1;
        }
      } catch (error) {
        failures.push(String(error));
      }
    }
    if (failures.length > 0) {
      throw new Error(`Lifecycle listener cleanup failed: ${failures.join("; ")}`);
    }
    return cleaned;
  };

  try {
    if (target === "overlay") {
      state.unlisteners.push(
        await event.listen("live-overlay-session", ({ payload }) => state.sessions.push(payload)),
      );
      state.unlisteners.push(
        await event.listen("live-level", ({ payload }) => state.levels.push(payload)),
      );
    } else {
      state.unlisteners.push(
        await event.listen("live-session-saved", ({ payload }) => state.saved.push(payload)),
      );
    }
    return state.unlisteners.length;
  } catch (registrationError) {
    try {
      await state.cleanup();
    } catch (cleanupError) {
      throw new Error(
        `${String(registrationError)}; partial listener cleanup also failed: ${String(cleanupError)}`,
      );
    }
    throw registrationError;
  }
}

export async function waitForTask8bSavedEvent(_tauri, options = {}) {
  const expectedCount = options.expectedCount ?? 1;
  const pollIntervalMs = options.pollIntervalMs ?? 25;
  const timeoutMs = options.timeoutMs ?? 5_000;
  if (!Number.isInteger(expectedCount) || expectedCount < 1) {
    throw new Error("Saved-event barrier expectedCount must be a positive integer.");
  }
  if (!Number.isFinite(pollIntervalMs) || pollIntervalMs <= 0) {
    throw new Error("Saved-event barrier pollIntervalMs must be positive.");
  }
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    throw new Error("Saved-event barrier timeoutMs must be positive.");
  }

  const deadline = Date.now() + timeoutMs;
  while (true) {
    const state = globalThis.__yapTask8bLifecycle;
    if (!state || !Array.isArray(state.saved)) {
      throw new Error("Task 8b lifecycle state is unavailable while waiting for a saved event.");
    }
    if (state.saved.length >= expectedCount) {
      return {
        levels: [...state.levels],
        saved: [...state.saved],
        sessions: [...state.sessions],
      };
    }

    const remainingMs = deadline - Date.now();
    if (remainingMs <= 0) {
      throw new Error(
        `Timed out waiting for ${expectedCount} saved event(s); received ${state.saved.length}.`,
      );
    }
    await new Promise((resolve) => {
      setTimeout(resolve, Math.min(pollIntervalMs, remainingMs));
    });
  }
}
