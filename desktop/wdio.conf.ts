import path from "node:path";

const binaryName = process.platform === "win32" ? "desktop.exe" : "desktop";
const appBinaryPath =
  process.env.APP_BINARY ?? path.resolve("src-tauri", "target", "debug", binaryName);

export const config = {
  bail: 0,
  baseUrl: "http://localhost:4445",
  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": {
        application: appBinaryPath,
      },
    },
  ],
  connectionRetryCount: 1,
  connectionRetryTimeout: 120_000,
  framework: "mocha",
  logLevel: "info",
  maxInstances: 1,
  mochaOpts: {
    timeout: 60_000,
    ui: "bdd",
  },
  outputDir: "./test-results/wdio",
  reporters: ["spec"],
  runner: "local",
  services: [
    [
      "@wdio/tauri-service",
      {
        appBinaryPath,
        backendLogLevel: "info",
        captureBackendLogs: true,
        captureFrontendLogs: true,
        driverProvider: "embedded",
        embeddedPort: 4445,
        frontendLogLevel: "warn",
      },
    ],
  ],
  specs: ["./wdio/**/*.spec.js"],
  waitforTimeout: 10_000,
  async afterTest(_test, _context, result) {
    if (result.error) {
      const safeName = result.error.message.replace(/[^a-z0-9]+/gi, "-").slice(0, 80);
      await browser.saveScreenshot(`./test-results/wdio/failure-${Date.now()}-${safeName}.png`);
    }
  },
};
