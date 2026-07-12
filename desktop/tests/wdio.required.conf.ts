import path from "node:path";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

import { config as baseConfig } from "./wdio.conf.ts";
import { assertRequiredSpecPolicy } from "./wdio/required-spec-policy.mjs";

const testsRoot = path.dirname(fileURLToPath(import.meta.url));
const requiredSpecs = [
  path.join(testsRoot, "wdio", "smoke.spec.js"),
  path.join(testsRoot, "wdio", "live-overlay.spec.js"),
];
for (const spec of requiredSpecs) {
  assertRequiredSpecPolicy(readFileSync(spec, "utf8"), spec);
}

export const config = {
  ...baseConfig,
  bail: 1,
  logLevel: "info",
  mochaOpts: {
    ...baseConfig.mochaOpts,
    forbidOnly: true,
    forbidPending: true,
  },
  outputDir: path.join(testsRoot, "results", "wdio-required"),
  specs: requiredSpecs,
};
