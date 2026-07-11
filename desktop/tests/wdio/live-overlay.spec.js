import {
  assertOwnedSavedSession,
  assertRecordingRootEmpty,
  classifyNativeReadiness,
  listRecordingArtifacts,
  registerTask8bLifecycleListeners,
  waitForTask8bSavedEvent,
} from "./task-8b-helpers.js";

const lifecycleAssertions = [
  "overlay-context start and stop without main-window UI interaction",
  "armed/listening/speaking -> saving -> idle lifecycle ordering",
  "live-level delivery",
  "exactly one canonical live-session-saved event",
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
  if (!(await switchToWindow("live-overlay"))) return { levels: [], saved: [], sessions: [] };
  return browser.tauri.execute(() => {
    const state = globalThis.__yapTask8bLifecycle;
    return state
      ? { levels: state.levels, saved: state.saved, sessions: state.sessions }
      : { levels: [], saved: [], sessions: [] };
  });
}

async function cleanupLifecycle(runStartedAtMs) {
  const errors = [];
  const listenerCleanupCounts = [];
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
    await browser.tauri.execute(({ core }) => core.invoke("stop_live_session"));
    stopResolved = true;
  });

  if (stopResolved && Number.isFinite(runStartedAtMs)) {
    await attempt("saved-event barrier failed", async () => {
      if (!(await switchToWindow("live-overlay"))) {
        throw new Error("Live overlay closed before the saved event could be observed.");
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

  await attempt("first listener cleanup failed", async () => {
    if (!(await switchToWindow("live-overlay"))) {
      listenerCleanupCounts.push(0);
      return;
    }
    listenerCleanupCounts.push(await browser.tauri.execute(() =>
      globalThis.__yapTask8bLifecycle?.cleanup?.() ?? 0));
  });
  await attempt("second listener cleanup failed", async () => {
    if (!(await switchToWindow("live-overlay"))) {
      listenerCleanupCounts.push(0);
      return;
    }
    listenerCleanupCounts.push(await browser.tauri.execute(() =>
      globalThis.__yapTask8bLifecycle?.cleanup?.() ?? 0));
  });

  await attempt("owned recording deletion failed", async () => {
    if (saved.length === 0) return;
    const uniqueNames = new Set(saved.map(({ name }) => name));
    if (saved.length !== 1 || uniqueNames.size !== 1) {
      errors.push(new Error(`Expected one saved event during cleanup, received ${saved.length}.`));
    }
    const candidate = saved[0];
    const owned = assertOwnedSavedSession(candidate, recordingRoot, {
      runStartedAtMs,
    });
    await browser.tauri.switchWindow("main");
    await browser.tauri.execute(
      ({ core }, sessionId) => core.invoke("delete_saved_live_session", { sessionId }),
      owned.sessionId,
    );
  });

  await attempt("isolated recording root was not empty after cleanup", async () => {
    assertRecordingRootEmpty(recordingRoot);
  });
  return { errors, listenerCleanupCounts, saved };
}

describe("Yap live overlay window", () => {
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
    if (errors.length > 0) throw new AggregateError(errors, "Task 8b afterEach cleanup failed");
  });

  it("captures and saves one session entirely from the overlay context", async function () {
    const readiness = classifyNativeReadiness(await nativeReadiness());
    if (readiness.action === "skip") {
      console.warn(
        `[Task 8b native skip] ${readiness.reason}. Unproven assertions: ${lifecycleAssertions.join("; ")}`,
      );
      this.skip();
    }

    let primaryError;
    let runStartedAtMs;
    let teardown;
    try {
      await showIdleOverlay();
      await browser.tauri.switchWindow("live-overlay");
      expect(await browser.tauri.execute(registerTask8bLifecycleListeners)).toBe(3);
      expect(await browser.tauri.execute(() =>
        globalThis.__yapTask8bLifecycle.saved.length)).toBe(0);
      assertRecordingRootEmpty(recordingRoot);

      expect(await browser.getWindowHandle()).toBe("live-overlay");
      await browser.tauri.execute(() => {
        const sensor = document.querySelector('[data-overlay-surface="sensor"]');
        sensor.dispatchEvent(new MouseEvent("mouseover", {
          bubbles: true,
          clientX: 130,
          clientY: 3,
        }));
      });
      await browser.waitUntil(async () => browser.tauri.execute(() =>
        document.querySelector("[data-overlay-surface]")?.getAttribute("data-overlay-surface") === "peek"));
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
        return state.saved.length >= 1
          && state.sessions.some(({ status }) => status === "saving")
          && state.sessions.some(({ status }) => status === "idle");
      }), {
        interval: 100,
        timeout: 20_000,
        timeoutMsg: "live session did not save and return to idle",
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
      expect(evidence.levels.some(({ status }) => ["listening", "speaking"].includes(status))).toBe(true);
      expect(evidence.saved).toHaveLength(1);
      const owned = assertOwnedSavedSession(evidence.saved[0], recordingRoot, {
        runStartedAtMs,
      });
      expect(owned.artifactNames).toHaveLength(5);

      await browser.pause(2_750);
      const surface = await browser.tauri.execute(() =>
        document.querySelector("[data-overlay-surface]")?.getAttribute("data-overlay-surface"));
      expect(surface).toBe("sensor");
      const compact = await browser.tauri.execute(() => {
        const bounds = document.querySelector('[data-overlay-surface="sensor"]').getBoundingClientRect();
        return { height: bounds.height, width: bounds.width };
      });
      expect(compact.width).toBeLessThanOrEqual(260);
      expect(compact.height).toBeLessThanOrEqual(8);
      expect(await browser.tauri.execute(() =>
        globalThis.__yapTask8bLifecycle.saved.length)).toBe(1);
    } catch (error) {
      primaryError = error;
    } finally {
      teardown = await cleanupLifecycle(runStartedAtMs);
    }

    const errors = [primaryError, ...teardown.errors].filter(Boolean);
    if (errors.length > 0) throw new AggregateError(errors, "Task 8b lifecycle evidence failed");
    expect(teardown.saved).toHaveLength(1);
    expect(teardown.listenerCleanupCounts).toEqual([3, 0]);
    assertRecordingRootEmpty(recordingRoot);
  });

  // Tauri does not expose a cross-platform skip-taskbar/Alt-Tab readback command here.
  // These probes cover the enforceable surface: compact size, unfocused/non-closable state,
  // close-request survival, and command denial from the overlay webview.
  it("opens as a compact system overlay and refuses direct close", async () => {
    await showIdleOverlay();

    const overlay = await browser.tauri.execute(async ({ core }) => {
      const label = "live-overlay";
      const inner = await core.invoke("plugin:window|inner_size", { label });
      const outer = await core.invoke("plugin:window|outer_size", { label });
      return {
        closable: await core.invoke("plugin:window|is_closable", { label }),
        focused: await core.invoke("plugin:window|is_focused", { label }),
        inner,
        outer,
        scaleFactor: await core.invoke("plugin:window|scale_factor", { label }),
        visible: await core.invoke("plugin:window|is_visible", { label }),
      };
    });
    const logicalInner = {
      height: overlay.inner.height / overlay.scaleFactor,
      width: overlay.inner.width / overlay.scaleFactor,
    };
    const logicalOuter = {
      height: overlay.outer.height / overlay.scaleFactor,
      width: overlay.outer.width / overlay.scaleFactor,
    };
    expect(overlay.visible).toBe(true);
    expect(overlay.focused).toBe(false);
    expect(overlay.closable).toBe(false);
    expect(logicalInner.width).toBeLessThanOrEqual(260);
    expect(logicalInner.height).toBeLessThanOrEqual(60);
    expect(logicalOuter.width).toBeLessThanOrEqual(300);
    expect(logicalOuter.height).toBeLessThanOrEqual(80);
    expect(listRecordingArtifacts(recordingRoot)).toEqual([]);
  });

  it("allows live status, rejects privileged commands, and survives close attempts", async () => {
    await showIdleOverlay();
    await browser.tauri.switchWindow("live-overlay");

    const authorization = await browser.tauri.execute(async ({ core }) => {
      const live = await core.invoke("live_status");
      let setup;
      try {
        await core.invoke("setup_status");
        setup = { ok: true, message: "" };
      } catch (error) {
        setup = { ok: false, message: String(error) };
      }
      let file;
      try {
        await core.invoke("open_app_path", { path: "C:\\not-a-yap-file.txt" });
        file = { ok: true, message: "" };
      } catch (error) {
        file = { ok: false, message: String(error) };
      }
      return { file, live, setup };
    });
    expect(typeof authorization.live.status).toBe("string");
    expect(authorization.setup.ok).toBe(false);
    expect(authorization.setup.message).toContain("Command is not available from this window.");
    expect(authorization.file.ok).toBe(false);
    expect(authorization.file.message).toContain(
      "This file action is only available from the main window.",
    );

    const closeAttempt = await browser.tauri.execute(async ({ core }) => {
      try {
        await core.invoke("plugin:window|close", { label: "live-overlay" });
        return { ok: true, message: "" };
      } catch (error) {
        return { ok: false, message: String(error) };
      }
    });
    expect(closeAttempt.ok).toBe(true);
    await browser.pause(250);

    const windows = await browser.tauri.listWindows();
    expect(windows).toContain("main");
    expect(windows).toContain("live-overlay");
    expect(await browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|is_visible", { label: "live-overlay" }))).toBe(true);
    expect(listRecordingArtifacts(recordingRoot)).toEqual([]);
  });

  it("keeps main alive when closed and restores it from the overlay", async () => {
    await showIdleOverlay();
    await browser.tauri.switchWindow("main");

    await browser.tauri.execute(({ core }) => core.invoke("plugin:window|close", { label: "main" }));
    await browser.waitUntil(async () => {
      const windows = await browser.tauri.listWindows();
      if (!windows.includes("main")) return false;
      const visible = await browser.tauri.execute(({ core }) =>
        core.invoke("plugin:window|is_visible", { label: "main" }));
      return !visible;
    }, {
      interval: 50,
      timeout: 5_000,
      timeoutMsg: "main window did not remain hidden after a close request",
    });
    expect(await browser.tauri.listWindows()).toContain("main");

    await browser.tauri.switchWindow("live-overlay");
    await browser.tauri.execute(({ core }) =>
      core.invoke("show_main_workspace", { workspace: "home" }));
    await browser.waitUntil(async () => browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|is_visible", { label: "main" })), {
      interval: 50,
      timeout: 5_000,
      timeoutMsg: "overlay command did not restore the main window",
    });
    expect(await browser.tauri.listWindows()).toContain("main");
  });
});
