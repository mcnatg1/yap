import {
  assertRecordingRootEmpty,
  listRecordingArtifacts,
} from "./task-8b-artifacts.js";
import {
  closeMainToTray,
  cycleIdleOverlay,
  recordingRoot,
  sampleWdioProcessTree,
  showIdleOverlay,
  withMainWindowRestored,
} from "./live-overlay-window-fixture.js";


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

  // Tauri does not expose a cross-platform skip-taskbar/Alt-Tab readback command here.
  // These probes cover the enforceable surface: exact visible size, unfocused/non-closable state,
  // close-request survival, and command denial from the overlay webview.
  it("opens as a compact system overlay and refuses direct close", async () => {
    await showIdleOverlay();

    await browser.tauri.switchWindow("live-overlay");
    const scaleFactor = await browser.tauri.execute(() => window.devicePixelRatio);
    if (!Number.isFinite(scaleFactor) || scaleFactor <= 0) {
      throw new Error(`Overlay reported invalid devicePixelRatio ${scaleFactor}.`);
    }
    await browser.tauri.switchWindow("main");

    const overlay = await browser.tauri.execute(async ({ core }) => {
      const label = "live-overlay";
      const inner = await core.invoke("plugin:window|inner_size", { label });
      const outer = await core.invoke("plugin:window|outer_size", { label });
      return {
        closable: await core.invoke("plugin:window|is_closable", { label }),
        focused: await core.invoke("plugin:window|is_focused", { label }),
        inner,
        outer,
        visible: await core.invoke("plugin:window|is_visible", { label }),
      };
    });
    const logicalInner = {
      height: overlay.inner.height / scaleFactor,
      width: overlay.inner.width / scaleFactor,
    };
    const logicalOuter = {
      height: overlay.outer.height / scaleFactor,
      width: overlay.outer.width / scaleFactor,
    };
    expect(overlay.visible).toBe(true);
    expect(overlay.focused).toBe(false);
    expect(overlay.closable).toBe(false);
    expect(logicalInner.width).toBeCloseTo(104, 1);
    expect(logicalInner.height).toBeCloseTo(40, 1);
    expect(logicalOuter.width).toBeCloseTo(104, 1);
    expect(logicalOuter.height).toBeCloseTo(40, 1);
    expect(listRecordingArtifacts(recordingRoot)).toEqual([]);
  });

  it("reuses one native window whose bounds equal each visible island surface", async () => {
    await showIdleOverlay();
    await browser.tauri.switchWindow("live-overlay");

    const scaleFactor = await browser.tauri.execute(() => window.devicePixelRatio);
    if (!Number.isFinite(scaleFactor) || scaleFactor <= 0) {
      throw new Error(`Overlay reported invalid devicePixelRatio ${scaleFactor}.`);
    }

    const labelsBefore = await browser.tauri.listWindows();
    expect(labelsBefore.filter((label) => label === "live-overlay")).toHaveLength(1);
    await browser.tauri.execute(() => {
      const root = document.querySelector('[data-overlay-surface="collapsed"]');
      root.dispatchEvent(new PointerEvent("pointerover", {
        bubbles: true,
        clientX: 52,
        clientY: 20,
      }));
    });
    await browser.waitUntil(async () => browser.tauri.execute(() => {
      const root = document.querySelector('[data-overlay-surface="expanded"]');
      const island = document.querySelector('[data-testid="live-overlay-island"]');
      if (!root || !island) return false;
      const rootBox = root.getBoundingClientRect();
      const islandBox = island.getBoundingClientRect();
      return rootBox.width === islandBox.width
        && rootBox.height === islandBox.height;
    }), {
      interval: 25,
      timeout: 5_000,
      timeoutMsg: "expanded webview did not converge to the visible island",
    });
    await browser.tauri.switchWindow("main");
    await browser.waitUntil(async () => browser.tauri.execute(async ({ core }, scale) => {
      const inner = await core.invoke("plugin:window|inner_size", { label: "live-overlay" });
      return Math.abs(inner.width / scale - 180) <= 0.5
        && Math.abs(inner.height / scale - 88) <= 0.5;
    }, scaleFactor), {
      interval: 25,
      timeout: 5_000,
      timeoutMsg: "expanded native bounds did not converge to 180 by 88",
    });
    expect(await browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|is_focused", { label: "live-overlay" }))).toBe(false);
    expect((await browser.tauri.listWindows()).filter((label) => label === "live-overlay")).toHaveLength(1);

    await browser.tauri.switchWindow("live-overlay");
    await browser.tauri.execute(() => {
      const root = document.querySelector('[data-overlay-surface="expanded"]');
      root.dispatchEvent(new PointerEvent("pointerout", {
        bubbles: true,
        relatedTarget: document.body,
      }));
    });
    await browser.waitUntil(async () => browser.tauri.execute(() => {
      const root = document.querySelector('[data-overlay-surface="collapsed"]');
      const island = document.querySelector('[data-testid="live-overlay-island"]');
      if (!root || !island) return false;
      const rootBox = root.getBoundingClientRect();
      const islandBox = island.getBoundingClientRect();
      return rootBox.width === islandBox.width
        && rootBox.height === islandBox.height;
    }), {
      interval: 25,
      timeout: 5_000,
      timeoutMsg: "collapsed webview did not converge after the grace period",
    });
    await browser.tauri.switchWindow("main");
    await browser.waitUntil(async () => browser.tauri.execute(async ({ core }, scale) => {
      const inner = await core.invoke("plugin:window|inner_size", { label: "live-overlay" });
      return Math.abs(inner.width / scale - 104) <= 0.5
        && Math.abs(inner.height / scale - 40) <= 0.5;
    }, scaleFactor), {
      interval: 25,
      timeout: 5_000,
      timeoutMsg: "collapsed native bounds did not converge to 104 by 40",
    });
    expect(await browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|is_focused", { label: "live-overlay" }))).toBe(false);
    expect(listRecordingArtifacts(recordingRoot)).toEqual([]);
  });

  it("keeps repeated expand-collapse resource growth bounded", async () => {
    await showIdleOverlay();
    await browser.tauri.switchWindow("live-overlay");
    await cycleIdleOverlay();
    await cycleIdleOverlay();
    const before = await sampleWdioProcessTree();

    for (let iteration = 0; iteration < 20; iteration += 1) {
      await cycleIdleOverlay();
    }
    await browser.pause(500);
    const after = await sampleWdioProcessTree();

    expect(after.processCount).toBeLessThanOrEqual(before.processCount + 2);
    expect(after.workingSetBytes - before.workingSetBytes).toBeLessThanOrEqual(96 * 1024 * 1024);
    expect(after.cpuSeconds - before.cpuSeconds).toBeLessThanOrEqual(10);
    expect((await browser.tauri.listWindows()).filter((label) => label === "live-overlay")).toHaveLength(1);
    expect(listRecordingArtifacts(recordingRoot)).toEqual([]);
  });

  it("does not expose raw renderer shortcut mutation commands", async () => {
    await browser.tauri.switchWindow("main");
    const original = await browser.tauri.execute(({ core }) => core.invoke("live_status"));

    for (const command of ["set_live_hotkey", "set_live_paste_hotkey"]) {
      const result = await browser.tauri.execute(async ({ core }, unavailableCommand) => {
        try {
          await core.invoke(unavailableCommand, { hotkey: "Ctrl+Shift+Alt+F11" });
          return { message: "", ok: true };
        } catch (error) {
          return { message: String(error), ok: false };
        }
      }, command);
      expect(result.ok).toBe(false);
      expect(result.message.toLowerCase()).toContain("not found");
    }

    const unchanged = await browser.tauri.execute(({ core }) => core.invoke("live_status"));
    expect(unchanged.hotkey).toBe(original.hotkey);
    expect(unchanged.pasteHotkey).toBe(original.pasteHotkey);
  });

  it("allows only minimized overlay status, rejects privileged commands, and survives close attempts", async () => {
    await showIdleOverlay();
    await browser.tauri.switchWindow("live-overlay");

    const authorization = await browser.tauri.execute(async ({ core }) => {
      const live = await core.invoke("live_overlay_status");
      let fullLive;
      try {
        await core.invoke("live_status");
        fullLive = { ok: true, message: "" };
      } catch (error) {
        fullLive = { ok: false, message: String(error) };
      }
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
      return { file, fullLive, live, setup };
    });
    expect(typeof authorization.live.status).toBe("string");
    expect(authorization.live.hasFinalText).toBe(false);
    expect(authorization.live).not.toHaveProperty("partialText");
    expect(authorization.live).not.toHaveProperty("finalText");
    expect(authorization.live).not.toHaveProperty("inputDeviceId");
    expect(authorization.live).not.toHaveProperty("inputDeviceLabel");
    expect(authorization.fullLive.ok).toBe(false);
    expect(authorization.fullLive.message).toContain("Command is not available from this window.");
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
    await withMainWindowRestored(async () => {
      await closeMainToTray();

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

  it("restores main and preserves the probe error after a hidden-state failure", async () => {
    await showIdleOverlay();
    const expectedError = new Error("simulated close-to-tray probe failure");
    let observedError;

    try {
      await withMainWindowRestored(async () => {
        await closeMainToTray();
        throw expectedError;
      });
    } catch (error) {
      observedError = error;
    }

    expect(observedError).toBe(expectedError);
    expect(await browser.tauri.listWindows()).toContain("main");
    expect(await browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|is_visible", { label: "main" }))).toBe(true);
  });
});
