import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export async function runWindowAction(action: "minimize" | "toggleMaximize" | "close") {
  if (!isTauri()) return;

  const window = getCurrentWindow();
  try {
    if (action === "minimize") await window.minimize();
    if (action === "toggleMaximize") await window.toggleMaximize();
    if (action === "close") await window.close();
  } catch {
    // ponytail: preview/dev without window permissions should not break the UI.
  }
}

export async function openDevtools() {
  if (!isTauri()) return;

  try {
    await invoke("open_devtools");
  } catch {
    // ponytail: browser preview should not care about native devtools.
  }
}
