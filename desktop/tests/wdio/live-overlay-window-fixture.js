import { execFile } from "node:child_process";
import path from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";


const execFileAsync = promisify(execFile);
const mainWindowTitle = "Yap";
const minMainWindowWidth = Math.floor(1122 * 0.7);
const minMainWindowHeight = Math.floor(740 * 0.7);
const nativeWindowRecoveryModule = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "native-window-recovery.psm1",
);

export const recordingRoot = process.env.YAP_LIVE_RECORDINGS_DIR;
if (!recordingRoot) throw new Error("WDIO requires an isolated YAP_LIVE_RECORDINGS_DIR.");

function powerShellLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

async function showMainWindowNatively() {
  const appPid = await resolveWdioAppPid();
  const script = `
$ErrorActionPreference = "Stop"
Import-Module -Name ${powerShellLiteral(nativeWindowRecoveryModule)} -Force
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
$null = Wait-WdioUniqueWindowCandidate -Probe {
    [WdioNativeWindow]::ShowMainWindowForProcess(
        [uint32]${appPid},
        ${JSON.stringify(mainWindowTitle)},
        ${minMainWindowWidth},
        ${minMainWindowHeight}
    )
} -Description "main Yap window for WDIO app PID ${appPid}" -MaxAttempts 50 -PollIntervalMilliseconds 100
`;
  await execFileAsync(
    "pwsh.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    { timeout: 10_000, windowsHide: true },
  );
}

async function resolveWdioAppPid() {
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
  return appPid;
}

export async function sampleWdioProcessTree() {
  const appPid = await resolveWdioAppPid();
  const script = `
$ErrorActionPreference = "Stop"
$all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId)
$ids = @([uint32]${appPid})
do {
  $children = @($all | Where-Object {
    $ids -contains [uint32]$_.ParentProcessId -and $ids -notcontains [uint32]$_.ProcessId
  } | ForEach-Object { [uint32]$_.ProcessId })
  if ($children.Count -eq 0) { break }
  $ids += $children
} while ($true)
$processes = @(Get-Process -Id ($ids | Sort-Object -Unique) -ErrorAction SilentlyContinue)
[pscustomobject]@{
  cpuSeconds = [double](($processes | Measure-Object -Property CPU -Sum).Sum)
  processCount = [int]$processes.Count
  workingSetBytes = [int64](($processes | Measure-Object -Property WorkingSet64 -Sum).Sum)
} | ConvertTo-Json -Compress
`;
  const { stdout } = await execFileAsync(
    "pwsh.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    { timeout: 15_000, windowsHide: true },
  );
  return JSON.parse(stdout.trim());
}

async function restoreMainWindow() {
  let mainVisible = false;
  let tauriRecoveryError;

  try {
    const windows = await browser.tauri.listWindows();
    if (!windows.includes("main")) {
      throw new Error("the main Tauri window no longer exists");
    }

    try {
      await browser.tauri.switchWindow("main");
      mainVisible = await browser.tauri.execute(({ core }) =>
        core.invoke("plugin:window|is_visible", { label: "main" }));
    } catch {
      mainVisible = false;
    }

    if (!mainVisible) {
      await browser.tauri.switchWindow("live-overlay");
      await browser.tauri.execute(({ core }) =>
        core.invoke("show_main_workspace", { workspace: "home" }));
      await browser.waitUntil(async () => browser.tauri.execute(({ core }) =>
        core.invoke("plugin:window|is_visible", { label: "main" })), {
        interval: 50,
        timeout: 5_000,
        timeoutMsg: "Tauri cleanup did not restore the main window",
      });
      mainVisible = true;
    }
  } catch (error) {
    tauriRecoveryError = error;
  }

  if (!mainVisible) {
    try {
      await showMainWindowNatively();
    } catch (nativeRecoveryError) {
      throw new AggregateError(
        [tauriRecoveryError, nativeRecoveryError].filter(Boolean),
        "Tauri and native main-window recovery both failed",
      );
    }
  }

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

export async function withMainWindowRestored(probe) {
  let probeFailed = false;

  try {
    return await probe();
  } catch (error) {
    probeFailed = true;
    throw error;
  } finally {
    try {
      await restoreMainWindow();
    } catch (cleanupError) {
      if (!probeFailed) throw cleanupError;
      console.error(
        "Main-window restoration also failed; preserving the close-to-tray probe error:",
        cleanupError,
      );
    }
  }
}

export async function closeMainToTray() {
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

export async function showIdleOverlay() {
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

export async function cycleIdleOverlay() {
  await browser.waitUntil(async () => browser.tauri.execute(() =>
    document.querySelector('[data-overlay-surface="collapsed"]') !== null), {
    interval: 20,
    timeout: 2_000,
    timeoutMsg: "collapsed overlay surface was unavailable before stability sampling",
  });
  await browser.tauri.execute(() => {
    const root = document.querySelector('[data-overlay-surface="collapsed"]');
    if (!root) throw new Error("collapsed overlay surface disappeared before pointerover");
    root.dispatchEvent(new PointerEvent("pointerover", { bubbles: true }));
  });
  await browser.waitUntil(async () => browser.tauri.execute(() =>
    document.querySelector("[data-overlay-surface]")?.getAttribute("data-overlay-surface") === "expanded"), {
    interval: 20,
    timeout: 2_000,
    timeoutMsg: "overlay did not expand during repeated stability sampling",
  });
  await browser.tauri.execute(() => {
    const root = document.querySelector('[data-overlay-surface="expanded"]');
    if (!root) throw new Error("expanded overlay surface disappeared before pointerout");
    root.dispatchEvent(new PointerEvent("pointerout", {
      bubbles: true,
      relatedTarget: document.body,
    }));
  });
  await browser.waitUntil(async () => browser.tauri.execute(() =>
    document.querySelector("[data-overlay-surface]")?.getAttribute("data-overlay-surface") === "collapsed"), {
    interval: 20,
    timeout: 2_000,
    timeoutMsg: "overlay did not collapse during repeated stability sampling",
  });
}
