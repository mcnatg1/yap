import {
  assertOwnedSavedSession,
  assertRecordingRootEmpty,
  classifyNativeReadiness,
  registerTask8bLifecycleListeners,
  waitForTask8bSavedEvent,
} from "./task-8b-helpers.js";

const lifecycleAssertions = [
  "overlay-context start and stop without main-window UI interaction",
  "armed/listening/speaking -> saving -> idle lifecycle ordering",
  "live-level delivery",
  "exactly one canonical main-window live-session-saved event",
  "no saved-path or transcript payload delivery to the overlay",
  "compact idle overlay after the success dwell",
  "idempotent listener cleanup and unregistration",
];

const recordingRoot = process.env.YAP_LIVE_RECORDINGS_DIR;
if (!recordingRoot) throw new Error("WDIO requires an isolated YAP_LIVE_RECORDINGS_DIR.");

async function switchToWindow(label) {
  const windows = await browser.tauri.listWindows();
  if (!windows.includes(label)) return false;
  await browser.tauri.switchWindow(label);
  return true;
}

async function showIdleOverlay() {
  await browser.tauri.switchWindow("main");
  const status = await browser.tauri.execute(({ core }) => core.invoke("live_status"));
  if (status.status !== "idle") {
    throw new Error(`Idle overlay test started from unexpected live status: ${status.status}`);
  }
  await browser.tauri.execute(({ core }) => core.invoke("show_live_overlay"));
  await browser.waitUntil(async () => (await browser.tauri.listWindows()).includes("live-overlay"));
  await browser.tauri.switchWindow("live-overlay");
  await browser.waitUntil(async () => browser.tauri.execute(() =>
    document.querySelector('[data-overlay-surface="collapsed"]') !== null), {
    interval: 25,
    timeout: 5_000,
    timeoutMsg: "live overlay window existed before its collapsed surface was ready",
  });
  await browser.tauri.switchWindow("main");
}

async function nativeReadiness() {
  return browser.tauri.execute(async ({ core }) => {
    const model = await core.invoke("fallback_model_status");
    let deviceError = null;
    let devices = null;
    let preflight = null;
    try {
      devices = await core.invoke("list_input_devices");
    } catch (error) {
      deviceError = String(error);
    }
    if (devices?.length) preflight = await core.invoke("preflight_input_device");
    return { deviceError, devices, model, preflight };
  });
}

async function readLifecycleEvidence() {
  let overlay = { levels: [], sessions: [] };
  let saved = [];
  if (await switchToWindow("live-overlay")) {
    overlay = await browser.tauri.execute(() => {
      const state = globalThis.__yapTask8bLifecycle;
      return state
        ? { levels: state.levels, sessions: state.sessions }
        : { levels: [], sessions: [] };
    });
  }
  if (await switchToWindow("main")) {
    saved = await browser.tauri.execute(() =>
      globalThis.__yapTask8bLifecycle?.saved ?? []);
  }
  await switchToWindow("live-overlay");
  return { ...overlay, saved };
}

async function cleanupWindowListeners(label, counts) {
  if (!(await switchToWindow(label))) {
    counts.push(0, 0);
    return;
  }
  counts.push(await browser.tauri.execute(() =>
    globalThis.__yapTask8bLifecycle?.cleanup?.() ?? 0));
  counts.push(await browser.tauri.execute(() =>
    globalThis.__yapTask8bLifecycle?.cleanup?.() ?? 0));
}

async function readCurrentLifecycleState() {
  return browser.tauri.execute(() => {
    const state = globalThis.__yapTask8bLifecycle;
    return state
      ? { levels: state.levels, saved: state.saved, sessions: state.sessions }
      : { levels: [], saved: [], sessions: [] };
  });
}

async function cleanupLifecycle(runStartedAtMs) {
  const errors = [];
  const listenerCleanupCounts = { main: [], overlay: [] };
  let saved = [];
  let stopResolved = false;

  const attempt = async (label, operation) => {
    try {
      return await operation();
    } catch (error) {
      errors.push(new Error(`${label}: ${String(error)}`));
      return undefined;
    }
  };

  await attempt("final stop failed", async () => {
    const target = (await switchToWindow("live-overlay")) ? "live-overlay" : "main";
    if (target === "main") await browser.tauri.switchWindow("main");
    const command = target === "live-overlay" ? "stop_live_overlay_session" : "stop_live_session";
    await browser.tauri.execute(({ core }, name) => core.invoke(name), command);
    stopResolved = true;
  });

  if (stopResolved && Number.isFinite(runStartedAtMs)) {
    await attempt("saved-event barrier failed", async () => {
      if (!(await switchToWindow("main"))) {
        throw new Error("Main window closed before the saved event could be observed.");
      }
      await browser.tauri.execute(waitForTask8bSavedEvent, {
        expectedCount: 1,
        pollIntervalMs: 25,
        timeoutMs: 5_000,
      });
    });
  }

  await attempt("saved-event capture failed", async () => {
    saved = (await readLifecycleEvidence()).saved;
  });

  await attempt("overlay listener cleanup failed", async () =>
    cleanupWindowListeners("live-overlay", listenerCleanupCounts.overlay));
  await attempt("main listener cleanup failed", async () =>
    cleanupWindowListeners("main", listenerCleanupCounts.main));

  await attempt("owned recording deletion failed", async () => {
    if (saved.length === 0) return;
    const uniqueNames = new Set(saved.map(({ name }) => name));
    if (saved.length !== 1 || uniqueNames.size !== 1) {
      errors.push(new Error(`Expected one saved event during cleanup, received ${saved.length}.`));
    }
    const candidate = saved[0];
    const owned = assertOwnedSavedSession(candidate, recordingRoot, { runStartedAtMs });
    await browser.tauri.switchWindow("main");
    await browser.tauri.execute(
      ({ core }, identity) => core.invoke("delete_saved_live_session", identity),
      {
        expectedCaptureCommitPath: candidate.captureCommitPath,
        expectedOutputPath: candidate.outputPath,
        sessionId: owned.sessionId,
      },
    );
  });

  await attempt("isolated recording root was not empty after cleanup", async () => {
    assertRecordingRootEmpty(recordingRoot);
  });
  return { errors, listenerCleanupCounts, saved };
}

describe("Yap live overlay hardware capture", () => {
  let overlayWasEnabled;

  beforeEach(async () => {
    assertRecordingRootEmpty(recordingRoot);
    await browser.tauri.switchWindow("main");
    const view = await browser.tauri.execute(({ core }) => core.invoke("live_status"));
    if (view.status !== "idle") {
      throw new Error(`WDIO test began with a non-idle live session: ${view.status}`);
    }
    overlayWasEnabled = view.visibility === "enabled";
  });

  afterEach(async () => {
    const errors = [];
    try {
      await browser.tauri.switchWindow("main");
      const view = await browser.tauri.execute(({ core }) => core.invoke("live_status"));
      if (view.status !== "idle") {
        await browser.tauri.execute(({ core }) => core.invoke("stop_live_session"));
        errors.push(new Error(`Test cleanup found and stopped live status ${view.status}.`));
      }
    } catch (error) {
      errors.push(new Error(`Live-state restoration failed: ${String(error)}`));
    }
    try {
      await browser.tauri.switchWindow("main");
      await browser.tauri.execute(
        ({ core }, enabled) => core.invoke("set_live_overlay_enabled", { enabled }),
        overlayWasEnabled,
      );
    } catch (error) {
      errors.push(new Error(`Overlay preference restoration failed: ${String(error)}`));
    }
    try {
      assertRecordingRootEmpty(recordingRoot);
    } catch (error) {
      errors.push(error);
    }
    if (errors.length > 0) throw new AggregateError(errors, "Hardware capture cleanup failed");
  });

  it("captures and saves one session entirely from the overlay context", async function () {
    const readiness = classifyNativeReadiness(await nativeReadiness());
    if (readiness.action === "skip") {
      console.warn(
        `[Optional native hardware skip] ${readiness.reason}. Unproven assertions: ${lifecycleAssertions.join("; ")}`,
      );
      this.skip();
    }

    let primaryError;
    let runStartedAtMs;
    let teardown;
    try {
      await showIdleOverlay();
      await browser.tauri.switchWindow("main");
      expect(await browser.tauri.execute(
        registerTask8bLifecycleListeners,
        { target: "main" },
      )).toBe(1);
      await browser.tauri.switchWindow("live-overlay");
      expect(await browser.tauri.execute(
        registerTask8bLifecycleListeners,
        { target: "overlay" },
      )).toBe(2);
      expect(await browser.tauri.execute(() =>
        globalThis.__yapTask8bLifecycle.saved.length)).toBe(0);
      assertRecordingRootEmpty(recordingRoot);

      expect(await browser.getWindowHandle()).toBe("live-overlay");
      await browser.tauri.execute(() => {
        const island = document.querySelector('[data-overlay-surface="collapsed"]');
        island.dispatchEvent(new PointerEvent("pointerover", {
          bubbles: true,
          clientX: 52,
          clientY: 20,
        }));
      });
      await browser.waitUntil(async () => browser.tauri.execute(() =>
        document.querySelector("[data-overlay-surface]")?.getAttribute("data-overlay-surface") === "expanded"));
      const startButton = await browser.$('[aria-label="Start dictating"]');
      await startButton.waitForDisplayed();
      runStartedAtMs = Date.now();
      await startButton.click();

      await browser.waitUntil(async () => browser.tauri.execute(() => {
        const state = globalThis.__yapTask8bLifecycle;
        return state.sessions.some(({ status }) => ["armed", "listening", "speaking"].includes(status))
          && state.levels.length > 0;
      }), {
        interval: 100,
        timeout: 20_000,
        timeoutMsg: "live-session and live-level did not report active capture",
      });
      await browser.pause(750);

      const finishButton = await browser.$('[aria-label="Finish recording"]');
      await finishButton.waitForDisplayed();
      await finishButton.click();
      await browser.waitUntil(async () => browser.tauri.execute(() => {
        const state = globalThis.__yapTask8bLifecycle;
        return state.sessions.some(({ status }) => status === "saving")
          && state.sessions.some(({ status }) => status === "idle");
      }), {
        interval: 100,
        timeout: 20_000,
        timeoutMsg: "live session did not save and return to idle",
      });
      expect((await readCurrentLifecycleState()).saved).toHaveLength(0);
      await browser.tauri.switchWindow("main");
      await browser.tauri.execute(waitForTask8bSavedEvent, {
        expectedCount: 1,
        pollIntervalMs: 25,
        timeoutMs: 5_000,
      });

      const evidence = await readLifecycleEvidence();
      const statuses = evidence.sessions.map(({ status }) => status);
      const activeIndex = statuses.findIndex((status) => ["armed", "listening", "speaking"].includes(status));
      const savingIndex = statuses.indexOf("saving", activeIndex + 1);
      const idleIndex = statuses.indexOf("idle", savingIndex + 1);
      expect(activeIndex).toBeGreaterThanOrEqual(0);
      expect(savingIndex).toBeGreaterThan(activeIndex);
      expect(idleIndex).toBeGreaterThan(savingIndex);
      expect(evidence.sessions[idleIndex].error).toBeNull();
      expect(evidence.levels.length).toBeGreaterThan(0);
      expect(evidence.levels.some(({ level }) => Number.isFinite(level))).toBe(true);
      expect(evidence.saved).toHaveLength(1);
      expect(evidence.sessions.every((session) =>
        !("partialText" in session)
        && !("finalText" in session)
        && !("inputDeviceId" in session)
        && !("inputDeviceLabel" in session))).toBe(true);
      const owned = assertOwnedSavedSession(evidence.saved[0], recordingRoot, { runStartedAtMs });
      expect(owned.artifactNames).toHaveLength(5);

      await browser.pause(2_750);
      const surface = await browser.tauri.execute(() =>
        document.querySelector("[data-overlay-surface]")?.getAttribute("data-overlay-surface"));
      expect(surface).toBe("collapsed");
      const compact = await browser.tauri.execute(() => {
        const root = document.querySelector('[data-overlay-surface="collapsed"]').getBoundingClientRect();
        const island = document.querySelector('[data-testid="live-overlay-island"]').getBoundingClientRect();
        return {
          island: { height: island.height, width: island.width },
          root: { height: root.height, width: root.width },
        };
      });
      expect(compact.root.width).toBe(104);
      expect(compact.root.height).toBe(40);
      expect(compact.island).toEqual(compact.root);
      expect(await browser.tauri.execute(() =>
        globalThis.__yapTask8bLifecycle.saved.length)).toBe(0);
    } catch (error) {
      primaryError = error;
    } finally {
      teardown = await cleanupLifecycle(runStartedAtMs);
    }

    const errors = [primaryError, ...teardown.errors].filter(Boolean);
    if (errors.length > 0) throw new AggregateError(errors, "Hardware lifecycle evidence failed");
    expect(teardown.saved).toHaveLength(1);
    expect(teardown.listenerCleanupCounts).toEqual({ main: [1, 0], overlay: [2, 0] });
    assertRecordingRootEmpty(recordingRoot);
  });
});
