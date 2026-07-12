import path from "node:path";
import { fileURLToPath } from "node:url";

import { config as baseConfig } from "./wdio.conf.ts";

const testsRoot = path.dirname(fileURLToPath(import.meta.url));

export const config = {
  ...baseConfig,
  bail: 1,
  mochaOpts: {
    ...baseConfig.mochaOpts,
    forbidOnly: true,
  },
  outputDir: path.join(testsRoot, "results", "wdio-hardware"),
  specs: [path.join(testsRoot, "wdio", "live-overlay.hardware.spec.js")],
};
