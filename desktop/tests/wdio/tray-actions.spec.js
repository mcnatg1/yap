import { execFile } from "node:child_process";
import process from "node:process";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

async function findWdioAppProcessId() {
  const webdriverPort = Number(browser.options.port ?? 4445);
  if (!Number.isInteger(webdriverPort)) {
    throw new Error(`Cannot identify the WDIO app from WebDriver port ${browser.options.port}.`);
  }
  const { stdout } = await execFileAsync("netstat.exe", ["-ano", "-p", "tcp"], {
    timeout: 5_000,
    windowsHide: true,
  });
  const listenerPattern = new RegExp(
    `^\\s*TCP\\s+\\S+:${webdriverPort}\\s+\\S+\\s+LISTENING\\s+(\\d+)\\s*$`,
    "mi",
  );
  const processId = Number(stdout.match(listenerPattern)?.[1]);
  if (!Number.isInteger(processId) || processId <= 0) {
    throw new Error(`No WDIO app is listening on port ${webdriverPort}.`);
  }
  return processId;
}

function isProcessAlive(processId) {
  try {
    process.kill(processId, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    throw error;
  }
}

async function waitForProcessExit(processId, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!isProcessAlive(processId)) return;
    await delay(50);
  }
  throw new Error(`Tray quit left WDIO app process ${processId} alive after ${timeoutMs}ms.`);
}

describe("Yap shared tray dispatcher", () => {
  it("restores the hidden main window through the production tray action", async () => {
    await browser.tauri.switchWindow("main");
    await browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|close", { label: "main" }));
    await browser.waitUntil(async () => !await browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|is_visible", { label: "main" })), {
      interval: 50,
      timeout: 5_000,
      timeoutMsg: "main window did not hide before the tray restore probe",
    });

    await browser.tauri.execute(({ core }) =>
      core.invoke("wdio_dispatch_tray_action", { action: "show_app" }));
    await browser.waitUntil(async () => browser.tauri.execute(({ core }) =>
      core.invoke("plugin:window|is_visible", { label: "main" })), {
      interval: 50,
      timeout: 5_000,
      timeoutMsg: "shared tray dispatcher did not restore the main window",
    });

    const denied = await browser.tauri.execute(async ({ core }) => {
      try {
        await core.invoke("wdio_dispatch_tray_action", { action: "start_dictating" });
        return "";
      } catch (error) {
        return String(error);
      }
    });
    expect(denied).toContain("only the restore and quit tray actions");
  });

  it("quits the app through the production tray action", async () => {
    const processId = await findWdioAppProcessId();
    let bridgeClosedDuringQuit = false;
    try {
      await browser.tauri.execute(({ core }) =>
        core.invoke("wdio_dispatch_tray_action", { action: "quit" }));
    } catch (error) {
      bridgeClosedDuringQuit = true;
      console.info(`WDIO bridge closed during tray quit: ${String(error)}`);
    }

    await waitForProcessExit(processId);
    expect(isProcessAlive(processId)).toBe(false);
    if (bridgeClosedDuringQuit) {
      console.info("Tray quit terminated the app before the WDIO bridge returned.");
    }
  });
});
