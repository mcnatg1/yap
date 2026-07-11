import { existsSync, statSync } from "node:fs";

const lifecycleAssertions = [
  "overlay-context start and stop without main-window UI interaction",
  "armed/listening/speaking -> saving -> idle lifecycle ordering",
  "live-level delivery",
  "exactly one canonical live-session-saved event",
  "compact idle overlay after the success dwell",
  "idempotent listener cleanup and unregistration",
];

describe("Yap live overlay window", () => {
  it("captures and saves one session entirely from the overlay context", async function () {
    const environment = await browser.tauri.execute(async ({ core }) => {
      const model = await core.invoke("fallback_model_status");
      const devices = await core.invoke("list_input_devices");
      const preflight = devices.length
        ? await core.invoke("preflight_input_device")
        : null;
      return { devices, model, preflight };
    });

    const modelSkipStatuses = new Set(["missing", "disabled", "corrupted"]);
    let skipReason;
    if (modelSkipStatuses.has(environment.model.status)) {
      skipReason = `Nemotron model is ${environment.model.status}`;
    } else if (environment.devices.length === 0) {
      skipReason = "no input device was enumerated";
    } else if (environment.preflight?.status === "blocked") {
      const error = environment.preflight.error ?? "";
      if (error === "No input detected." || error.startsWith("Microphone access failed:")) {
        skipReason = `microphone preflight was unavailable: ${error}`;
      } else {
        throw new Error(`Unexpected microphone preflight failure: ${error || "unknown error"}`);
      }
    }

    if (skipReason) {
      console.warn(
        `[Task 8b native skip] ${skipReason}. Unproven assertions: ${lifecycleAssertions.join("; ")}`,
      );
      this.skip();
    }
    expect(environment.model.status).toBe("ready");
    expect(environment.preflight.status).toBe("idle");

    await browser.tauri.execute(({ core }) => core.invoke("show_live_overlay"));
    await browser.waitUntil(async () => (await browser.tauri.listWindows()).includes("live-overlay"));
    await browser.tauri.switchWindow("live-overlay");

    let savedSession;
    let listenersRegistered = false;
    try {
      const listenerCount = await browser.tauri.execute(async () => {
        const event = globalThis.__TAURI__.event;
        const state = {
          levels: [],
          saved: [],
          sessions: [],
          unlisteners: [],
        };
        globalThis.__yapTask8bLifecycle = state;
        state.unlisteners.push(
          await event.listen("live-session", ({ payload }) => state.sessions.push(payload)),
          await event.listen("live-level", ({ payload }) => state.levels.push(payload)),
          await event.listen("live-session-saved", ({ payload }) => state.saved.push(payload)),
        );
        state.cleanup = async () => {
          const unlisteners = state.unlisteners.splice(0);
          for (const unlisten of unlisteners) await unlisten();
          return unlisteners.length;
        };
        return state.unlisteners.length;
      });
      expect(listenerCount).toBe(3);
      listenersRegistered = true;

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

      const evidence = await browser.tauri.execute(() => {
        const state = globalThis.__yapTask8bLifecycle;
        return {
          levels: state.levels,
          saved: state.saved,
          sessions: state.sessions,
        };
      });
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

      savedSession = evidence.saved[0];
      const sessionId = savedSession.name.slice("live-".length);
      const normalizedSource = savedSession.sourcePath.replaceAll("\\", "/");
      const normalizedOutput = savedSession.outputPath.replaceAll("\\", "/");
      const normalizedCommit = savedSession.captureCommitPath?.replaceAll("\\", "/");
      expect(savedSession.name).toMatch(/^live-s-[0-9a-f]+-[0-9a-f]+-[0-9a-f]+$/);
      expect(normalizedSource).toMatch(new RegExp(`/live-${sessionId}\\.wav$`));
      expect(normalizedOutput).toMatch(new RegExp(`/live-${sessionId}\\.txt$`));
      expect(normalizedCommit).toMatch(new RegExp(`/live-${sessionId}\\.commit\\.json$`));
      expect(existsSync(savedSession.sourcePath)).toBe(true);
      expect(existsSync(savedSession.outputPath)).toBe(true);
      expect(existsSync(savedSession.captureCommitPath)).toBe(true);
      expect(statSync(savedSession.sourcePath).size).toBeGreaterThan(44);

      await browser.pause(2_750);
      const surface = await browser.tauri.execute(() =>
        document.querySelector("[data-overlay-surface]")?.getAttribute("data-overlay-surface"));
      expect(surface).toBe("sensor");
      const compact = await browser.tauri.execute(() => {
        const bounds = document.querySelector('[data-overlay-surface="sensor"]').getBoundingClientRect();
        return {
          height: bounds.height,
          width: bounds.width,
        };
      });
      expect(compact.width).toBeLessThanOrEqual(260);
      expect(compact.height).toBeLessThanOrEqual(8);
      expect(await browser.tauri.execute(() =>
        globalThis.__yapTask8bLifecycle.saved.length)).toBe(1);

      const firstCleanup = await browser.tauri.execute(() =>
        globalThis.__yapTask8bLifecycle.cleanup());
      const secondCleanup = await browser.tauri.execute(() =>
        globalThis.__yapTask8bLifecycle.cleanup());
      expect(firstCleanup).toBe(3);
      expect(secondCleanup).toBe(0);
      listenersRegistered = false;
    } finally {
      if (!savedSession && listenersRegistered) {
        savedSession = await browser.tauri.execute(() =>
          globalThis.__yapTask8bLifecycle.saved[0] ?? null);
      }
      if (listenersRegistered) {
        await browser.tauri.execute(() => globalThis.__yapTask8bLifecycle.cleanup());
        await browser.tauri.execute(() => globalThis.__yapTask8bLifecycle.cleanup());
      }
      await browser.tauri.execute(({ core }) => core.invoke("stop_live_session"));
      await browser.tauri.switchWindow("main");
      if (savedSession?.name.startsWith("live-")) {
        await browser.tauri.execute(({ core }, sessionId) =>
          core.invoke("delete_saved_live_session", { sessionId }), savedSession.name.slice("live-".length));
      }
    }
  });

  // Tauri does not expose a cross-platform skip-taskbar/Alt-Tab readback command here.
  // These probes cover the enforceable surface: compact size, unfocused/non-closable state,
  // close-request survival, and command denial from the overlay webview.
  it("opens as a compact system overlay and refuses direct close", async () => {
    await browser.tauri.execute(({ core }) => core.invoke("start_live_session", { activeCaptureMode: "toggle" }));
    await browser.waitUntil(async () => (await browser.tauri.listWindows()).includes("live-overlay"));

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

    await browser.tauri.execute(({ core }) => core.invoke("stop_live_session"));
  });

  it("rejects main-window file actions from the overlay and survives close attempts", async () => {
    await browser.tauri.execute(({ core }) => core.invoke("start_live_session", { activeCaptureMode: "toggle" }));
    await browser.waitUntil(async () => (await browser.tauri.listWindows()).includes("live-overlay"));

    await browser.tauri.switchWindow("live-overlay");
    const denied = await browser.tauri.execute(async ({ core }) => {
      try {
        await core.invoke("open_app_path", { path: "C:\\not-a-yap-file.txt" });
        return { ok: true, message: "" };
      } catch (error) {
        return { ok: false, message: String(error) };
      }
    });
    expect(denied.ok).toBe(false);
    expect(denied.message).toContain("This file action is only available from the main window.");

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

    const overlay = await browser.tauri.execute(({ core }) => core.invoke("plugin:window|is_visible", { label: "live-overlay" }));
    expect(overlay).toBe(true);

    await browser.tauri.switchWindow("main");
    await browser.tauri.execute(({ core }) => core.invoke("stop_live_session"));
  });
});
