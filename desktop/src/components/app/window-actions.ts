import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

export async function runWindowAction(action: "minimize" | "toggleMaximize" | "close") {
  if (!isTauri()) return;

  const window = getCurrentWindow();
  try {
    if (action === "minimize") await window.minimize();
    if (action === "toggleMaximize") await window.toggleMaximize();
    if (action === "close") await window.close();
  } catch (error) {
    console.warn(`Window action failed: ${action}`, error);
  }
}
