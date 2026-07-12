import { defineConfig, devices } from "@playwright/test";
import process from "node:process";

const port = 4176;

export default defineConfig({
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  testDir: "./polish-owner",
  testMatch: "*.pw.ts",
  timeout: 20_000,
  use: { baseURL: `http://127.0.0.1:${port}` },
  webServer: {
    command: `node ../node_modules/vite/bin/vite.js .. --host 127.0.0.1 --port ${port} --strictPort`,
    env: {
      ...process.env,
      VITE_ENABLE_DEVELOPMENT_POLISH: "true",
    },
    reuseExistingServer: false,
    timeout: 60_000,
    url: `http://127.0.0.1:${port}`,
  },
});
