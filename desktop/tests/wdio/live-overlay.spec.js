import { execFile } from "node:child_process";
import { promisify } from "node:util";

import {
  assertRecordingRootEmpty,
  listRecordingArtifacts,
} from "./task-8b-helpers.js";

const execFileAsync = promisify(execFile);
const mainWindowTitle = "Yap";
const minMainWindowWidth = Math.floor(1122 * 0.7);
const minMainWindowHeight = Math.floor(740 * 0.7);

const recordingRoot = process.env.YAP_LIVE_RECORDINGS_DIR;
if (!recordingRoot) throw new Error("WDIO requires an isolated YAP_LIVE_RECORDINGS_DIR.");

async function showMainWindowNatively() {
  const webdriverPort = Number(browser.options.port ?? 4445);
  if (!Number.isInteger(webdriverPort)) {
    throw new Error(`Cannot identify the WDIO app from WebDriver port ${browser.options.port}.`);
  }
  // Resolve the isolated app by its WebDriver listener before touching any HWND.
  const { stdout } = await execFileAsync(
    "netstat.exe",
    ["-ano", "-p", "tcp"],
    { timeout: 5_000, windowsHide: true },
  );
  const listenerPattern = new RegExp(
    `^\\s*TCP\\s+\\S+:${webdriverPort}\\s+\\S+\\s+LISTENING\\s+(\\d+)\\s*$`,
    "mi",
  );
  const appPid = Number(stdout.match(listenerPattern)?.[1]);
  if (!Number.isInteger(appPid) || appPid <= 0) {
    throw new Error(`No WDIO app is listening on port ${webdriverPort}.`);
  }
  const script = `
$ErrorActionPreference = "Stop"
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class WdioNativeWindow {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    private delegate bool EnumWindowsCallback(IntPtr window, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsCallback callback, IntPtr parameter);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);

    [DllImport("user32.dll")]
    private static extern IntPtr GetWindow(IntPtr window, uint command);

    [DllImport("user32.dll")]
    private static extern int GetWindowLongW(IntPtr window, int index);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowTextW(IntPtr window, StringBuilder text, int maxCount);

    [DllImport("user32.dll")]
    private static extern bool ShowWindowAsync(IntPtr window, int command);

    private const uint GetOwner = 4;
    private const int ExtendedStyle = -20;
    private const int ToolWindowStyle = 0x00000080;

    public static int ShowMainWindowForProcess(
        uint ownerPid,
        string expectedTitle,
        int minWidth,
        int minHeight
    ) {
        var candidates = new List<IntPtr>();
        bool enumerated = EnumWindows((window, parameter) => {
            uint windowPid;
            Rect rect;
            GetWindowThreadProcessId(window, out windowPid);
            if (windowPid != ownerPid || GetWindow(window, GetOwner) != IntPtr.Zero) return true;
            if ((GetWindowLongW(window, ExtendedStyle) & ToolWindowStyle) != 0) return true;

            var title = new StringBuilder(256);
            GetWindowTextW(window, title, title.Capacity);
            if (!String.Equals(title.ToString(), expectedTitle, StringComparison.Ordinal)) return true;
            if (!GetWindowRect(window, out rect)) return true;

            long width = Math.Max(0, rect.Right - rect.Left);
            long height = Math.Max(0, rect.Bottom - rect.Top);
            if (width < minWidth || height < minHeight) return true;
            candidates.Add(window);
            return true;
        }, IntPtr.Zero);
        if (!enumerated) throw new InvalidOperationException("Failed to enumerate WDIO app windows.");
        if (candidates.Count == 1) ShowWindowAsync(candidates[0], 5);
        return candidates.Count;
    }
}
'@
$candidateCount = [WdioNativeWindow]::ShowMainWindowForProcess(
    [uint32]${appPid},
    ${JSON.stringify(mainWindowTitle)},
    ${minMainWindowWidth},
    ${minMainWindowHeight}
)
if ($candidateCount -ne 1) {
    throw "Expected exactly one main Yap window for WDIO app PID ${appPid}; found $candidateCount."
}
`;
  await execFileAsync(
    "pwsh.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    { timeout: 10_000, windowsHide: true },
  );
}

async function restoreMainWindowNatively() {
  await showMainWindowNatively();
  await browser.tauri.switchWindow("main");
  await browser.tauri.execute(({ core }) =>
    core.invoke("show_main_workspace", { workspace: "home" }));
  await browser.waitUntil(async () => browser.tauri.execute(({ core }) =>
    core.invoke("plugin:window|is_visible", { label: "main" })), {
    interval: 50,
    timeout: 5_000,
    timeoutMsg: "native cleanup did not restore the main window",
  });
}

async function withMainWindowRestored(probe) {
  let probeFailed = false;

  try {
    return await probe();
  } catch (error) {
    probeFailed = true;
    throw error;
  } finally {
    try {
      await restoreMainWindowNatively();
    } catch (cleanupError) {
      if (!probeFailed) throw cleanupError;
      console.error(
        "Main-window restoration also failed; preserving the close-to-tray probe error:",
        cleanupError,
      );
    }
  }
}

async function closeMainToTray() {
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
