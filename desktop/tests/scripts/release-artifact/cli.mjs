import path from "node:path";

import {
  bindReleaseArtifact,
  prepareReleaseContext,
  sealReleaseArtifact,
} from "./core.mjs";

export async function runReleaseArtifactCli(args) {
  const { mode, values } = parseArguments(args);
  const repoRoot = path.resolve(values.get("--repo-root") ?? process.cwd());
  if (mode === "prepare") {
    const context = await prepareReleaseContext({
      commitSha: required(values, "--commit-sha"),
      contextPath: required(values, "--context-path"),
      releaseTag: required(values, "--release-tag"),
      repoRoot,
    });
    console.log(JSON.stringify(context));
    return;
  }
  if (mode === "seal") {
    const result = await sealReleaseArtifact({
      bundleDirectory: required(values, "--bundle-directory"),
      contextPath: required(values, "--context-path"),
      githubOutput: values.get("--github-output"),
      repoRoot,
      sealPath: required(values, "--seal-path"),
    });
    console.log(JSON.stringify(result.seal));
    return;
  }
  if (mode === "bind") {
    const result = await bindReleaseArtifact({
      bundleDirectory: required(values, "--bundle-directory"),
      contextPath: required(values, "--context-path"),
      expectedInstallerSha256: required(values, "--expected-installer-sha256"),
      githubOutput: values.get("--github-output"),
      metadataPath: required(values, "--metadata-path"),
      repoRoot,
      sealPath: required(values, "--seal-path"),
    });
    console.log(JSON.stringify(result.metadata));
    return;
  }
  throw new Error(`Unknown release-artifact mode: ${mode ?? "<missing>"}`);
}

function parseArguments(args) {
  const [mode, ...tokens] = args;
  const values = new Map();
  for (let index = 0; index < tokens.length; index += 2) {
    const name = tokens[index];
    const value = tokens[index + 1];
    if (!name?.startsWith("--") || value === undefined || value.startsWith("--")) {
      throw new Error(`Invalid release-artifact argument near ${name ?? "<end>"}.`);
    }
    if (values.has(name)) throw new Error(`Duplicate release-artifact argument: ${name}`);
    values.set(name, value);
  }
  return { mode, values };
}

function required(values, name) {
  const value = values.get(name);
  if (!value) throw new Error(`Missing required argument: ${name}`);
  return value;
}
