import path from "node:path";
import { pathToFileURL } from "node:url";

import { runReleaseArtifactCli } from "./release-artifact/cli.mjs";

export {
  bindReleaseArtifact,
  prepareReleaseContext,
  sealReleaseArtifact,
  validateReleaseCoordinates,
} from "./release-artifact/core.mjs";

const entryPoint = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : "";
if (entryPoint === import.meta.url) {
  runReleaseArtifactCli(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
