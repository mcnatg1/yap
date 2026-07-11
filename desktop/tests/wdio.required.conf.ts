import path from "node:path";
import { fileURLToPath } from "node:url";

import { config as baseConfig } from "./wdio.conf.ts";

const testsRoot = path.dirname(fileURLToPath(import.meta.url));

export const config = {
  ...baseConfig,
  bail: 1,
  logLevel: "trace",
  mochaOpts: {
    ...baseConfig.mochaOpts,
    forbidPending: true,
  },
  outputDir: path.join(testsRoot, "results", "wdio-required"),
  specs: [path.join(testsRoot, "wdio", "smoke.spec.js")],
};
