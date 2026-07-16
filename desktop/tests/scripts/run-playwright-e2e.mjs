import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { selectPlaywrightPort } from "./playwright-port.mjs";

const scriptPath = fileURLToPath(import.meta.url);
const desktopRoot = path.resolve(path.dirname(scriptPath), "..", "..");
const configPath = path.join(desktopRoot, "tests", "playwright.config.ts");
const playwrightCli = fileURLToPath(import.meta.resolve("@playwright/test/cli"));

export async function runPlaywrightE2e(args = process.argv.slice(2)) {
  const port = await selectPlaywrightPort(process.env.YAP_PLAYWRIGHT_PORT);
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      [playwrightCli, "test", "-c", configPath, ...args],
      {
        cwd: desktopRoot,
        env: {
          ...process.env,
          YAP_PLAYWRIGHT_PORT: String(port),
        },
        stdio: "inherit",
      },
    );
    child.once("error", reject);
    child.once("exit", (code) => resolve(code ?? 1));
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  process.exitCode = await runPlaywrightE2e();
}
