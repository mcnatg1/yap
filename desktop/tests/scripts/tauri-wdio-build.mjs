import { copyFile, rm } from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

import { resolvePackageManagerCommand } from "./package-manager-command.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const source = path.join(root, "tests", "wdio", "capabilities", "wdio.json");
const generated = path.join(root, "src-tauri", "capabilities", "wdio.generated.json");
const packageManager = resolvePackageManagerCommand({
  args: [
    "tauri",
    "build",
    "--debug",
    "--features",
    "wdio",
    "--config",
    "src-tauri/tauri.wdio.conf.json",
    "--no-bundle",
  ],
  nodeExecPath: process.execPath,
  npmExecPath: process.env.npm_execpath,
});

await rm(generated, { force: true });
await copyFile(source, generated);

try {
  const exitCode = await run(
    packageManager.command,
    packageManager.args,
  );
  process.exitCode = exitCode;
} finally {
  await rm(generated, { force: true });
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: root,
      env: {
        ...process.env,
        VITE_WDIO: "1",
      },
      stdio: "inherit",
    });

    child.on("error", reject);
    child.on("exit", (code) => resolve(code ?? 1));
  });
}
