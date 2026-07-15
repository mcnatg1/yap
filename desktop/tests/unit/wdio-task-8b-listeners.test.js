import { describe, expect, it } from "vitest";

import {
  registerTask8bLifecycleListeners,
  waitForTask8bSavedEvent,
} from "../wdio/task-8b-helpers.js";


describe("Task 8b transactional lifecycle listeners", () => {
  it("immediately unregisters earlier listeners when partial setup fails", async () => {
    const calls = [];
    const unlisten = () => calls.push("unlisten-live-session");
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          calls.push(name);
          if (name === "live-level") throw new Error("registration failed");
          return unlisten;
        },
      },
    };

    try {
      await expect(registerTask8bLifecycleListeners()).rejects.toThrow("registration failed");
      expect(calls).toEqual(["live-overlay-session", "live-level", "unlisten-live-session"]);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("unregisters all listeners exactly once", async () => {
    const unlistened = [];
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          return () => unlistened.push(name);
        },
      },
    };

    try {
      await expect(registerTask8bLifecycleListeners()).resolves.toBe(2);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(2);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(0);
      expect(unlistened).toEqual(["live-overlay-session", "live-level"]);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("keeps saved artifact events on a separate main-window listener", async () => {
    const priorTauri = globalThis.__TAURI__;
    let handler;
    globalThis.__TAURI__ = {
      event: {
        async listen(name, callback) {
          expect(name).toBe("live-session-saved");
          handler = callback;
          return () => undefined;
        },
      },
    };

    try {
      await expect(registerTask8bLifecycleListeners({}, { target: "main" })).resolves.toBe(1);
      handler({ payload: { name: "live-s-1-2-3" } });
      expect(globalThis.__yapTask8bLifecycle.saved).toEqual([{ name: "live-s-1-2-3" }]);
      expect(globalThis.__yapTask8bLifecycle.sessions).toEqual([]);
      expect(globalThis.__yapTask8bLifecycle.levels).toEqual([]);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("retains a rejecting unlistener for a later successful retry", async () => {
    let rejectOnceAttempts = 0;
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          if (name !== "live-overlay-session") return () => undefined;
          return async () => {
            rejectOnceAttempts += 1;
            if (rejectOnceAttempts === 1) throw new Error("unlisten retry required");
          };
        },
      },
    };

    try {
      await registerTask8bLifecycleListeners();
      await expect(globalThis.__yapTask8bLifecycle.cleanup())
        .rejects.toThrow("unlisten retry required");
      expect(globalThis.__yapTask8bLifecycle.unlisteners).toHaveLength(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(0);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("preserves failed registration cleanup handles for outer-finally recovery", async () => {
    let cleanupAttempts = 0;
    const priorTauri = globalThis.__TAURI__;
    globalThis.__TAURI__ = {
      event: {
        async listen(name) {
          if (name === "live-level") throw new Error("registration failed");
          return async () => {
            cleanupAttempts += 1;
            if (cleanupAttempts === 1) throw new Error("partial cleanup failed");
          };
        },
      },
    };

    try {
      await expect(registerTask8bLifecycleListeners())
        .rejects.toThrow(/registration failed.*partial cleanup failed/i);
      expect(globalThis.__yapTask8bLifecycle.unlisteners).toHaveLength(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(1);
      await expect(globalThis.__yapTask8bLifecycle.cleanup()).resolves.toBe(0);
    } finally {
      globalThis.__TAURI__ = priorTauri;
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("waits through delayed saved-event dispatch before returning evidence", async () => {
    globalThis.__yapTask8bLifecycle = { levels: [], saved: [], sessions: [] };
    const dispatch = setTimeout(() => {
      globalThis.__yapTask8bLifecycle.saved.push({ name: "live-s-1-2-3" });
    }, 10);

    try {
      await expect(waitForTask8bSavedEvent({}, {
        expectedCount: 1,
        pollIntervalMs: 1,
        timeoutMs: 100,
      })).resolves.toMatchObject({
        saved: [{ name: "live-s-1-2-3" }],
      });
    } finally {
      clearTimeout(dispatch);
      delete globalThis.__yapTask8bLifecycle;
    }
  });

  it("fails within the bounded saved-event deadline", async () => {
    globalThis.__yapTask8bLifecycle = { levels: [], saved: [], sessions: [] };
    try {
      await expect(waitForTask8bSavedEvent({}, {
        expectedCount: 1,
        pollIntervalMs: 1,
        timeoutMs: 10,
      })).rejects.toThrow(/timed out waiting for 1 saved event/i);
    } finally {
      delete globalThis.__yapTask8bLifecycle;
    }
  });
});
