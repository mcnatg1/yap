import { defineConfig, devices } from "@playwright/test";

const configuredPort = process.env.YAP_PLAYWRIGHT_PORT ?? "4174";
if (!/^\d+$/.test(configuredPort)) {
  throw new Error("YAP_PLAYWRIGHT_PORT must be an integer TCP port.");
}
const testPort = Number(configuredPort);
if (testPort < 1 || testPort > 65_535) {
  throw new Error("YAP_PLAYWRIGHT_PORT must be between 1 and 65535.");
}
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
