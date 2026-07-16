import { defineConfig, devices } from "@playwright/test";
import { parsePlaywrightPort } from "./scripts/playwright-port.mjs";

const testPort = parsePlaywrightPort(process.env.YAP_PLAYWRIGHT_PORT);
const testUrl = `http://127.0.0.1:${testPort}`;

export default defineConfig({
  expect: {
    timeout: 5_000,
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  outputDir: "./results/playwright",
  testDir: "./e2e",
  timeout: 20_000,
  use: {
    baseURL: testUrl,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "retain-on-failure",
  },
  webServer: {
    command: `pnpm dev --host 127.0.0.1 --port ${testPort} --strictPort`,
    reuseExistingServer: false,
    timeout: 60_000,
    url: testUrl,
  },
});
