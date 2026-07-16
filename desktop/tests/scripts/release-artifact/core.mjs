import { readFile } from "node:fs/promises";
import path from "node:path";

import {
  appendGitHubOutputs,
  assertArtifactSeal,
  assertSha256,
  hashFile,
  resolveInstaller,
  writeNewJson,
} from "./artifact-files.mjs";
import {
  assertReleaseCheckout,
  assertReleaseContext,
  collectBuildEnvironment,
  readReleaseVersion,
  validateReleaseCoordinates,
  validateReleaseVersion,
} from "./release-state.mjs";

export { validateReleaseCoordinates };

export async function prepareReleaseContext({
  commitSha,
  contextPath,
  releaseTag,
  repoRoot,
}) {
  validateReleaseCoordinates(commitSha, releaseTag);
  assertReleaseCheckout(repoRoot, commitSha, "preparation");
  const version = readReleaseVersion(repoRoot, commitSha);
  validateReleaseVersion(releaseTag, version);
  assertReleaseCheckout(repoRoot, commitSha, "preparation");
  const context = {
    schemaVersion: 3,
    releaseTag,
    commitSha,
    version,
  };
  await writeNewJson(contextPath, context);
  return context;
}

export async function sealReleaseArtifact({
  bundleDirectory,
  contextPath,
  githubOutput,
  repoRoot,
  sealPath,
}) {
  const context = JSON.parse(await readFile(contextPath, "utf8"));
  assertReleaseContext(context);
  assertReleaseCheckout(repoRoot, context.commitSha, "artifact sealing");

  const { installerPath, installerStat } = await resolveInstaller(
    bundleDirectory,
    context.version,
  );
  const sha256 = await hashFile(installerPath);
  assertReleaseCheckout(repoRoot, context.commitSha, "artifact sealing");

  const seal = {
    schemaVersion: 1,
    releaseTag: context.releaseTag,
    commitSha: context.commitSha,
    version: context.version,
    artifact: {
      fileName: path.basename(installerPath),
      sha256,
      sizeBytes: installerStat.size,
    },
  };
  await writeNewJson(sealPath, seal);

  if (githubOutput) {
    await appendGitHubOutputs(githubOutput, {
      installer_name: seal.artifact.fileName,
      installer_path: installerPath,
      installer_sha256: sha256,
      seal_path: path.resolve(sealPath),
      release_sha: context.commitSha,
      release_tag: context.releaseTag,
    });
  }

  return { installerPath, seal };
}

export async function bindReleaseArtifact({
  bundleDirectory,
  contextPath,
  expectedInstallerSha256,
  githubOutput,
  metadataPath,
  repoRoot,
  sealPath,
}) {
  const context = JSON.parse(await readFile(contextPath, "utf8"));
  assertReleaseContext(context);
  assertSha256(expectedInstallerSha256, "expected sealed installer");
  assertReleaseCheckout(repoRoot, context.commitSha, "artifact binding");
  const seal = JSON.parse(await readFile(sealPath, "utf8"));
  assertArtifactSeal(seal, context);
  if (seal.artifact.sha256 !== expectedInstallerSha256) {
    throw new Error("Mutable artifact seal differs from the immutable pre-smoke SHA-256.");
  }

  const { installerPath, installerStat } = await resolveInstaller(
    bundleDirectory,
    context.version,
  );
  const sha256 = await hashFile(installerPath);
  if (sha256 !== expectedInstallerSha256) {
    throw new Error("NSIS installer differs from the immutable pre-smoke SHA-256.");
  }
  if (path.basename(installerPath) !== seal.artifact.fileName) {
    throw new Error("NSIS installer name differs from the sealed pre-smoke artifact.");
  }
  if (installerStat.size !== seal.artifact.sizeBytes) {
    throw new Error("NSIS installer size differs from the sealed pre-smoke artifact.");
  }
  if (sha256 !== seal.artifact.sha256) {
    throw new Error("NSIS installer differs from its sealed pre-smoke SHA-256.");
  }

  const metadata = {
    ...context,
    artifact: {
      fileName: path.basename(installerPath),
      sha256,
      sizeBytes: installerStat.size,
    },
    buildEnvironment: collectBuildEnvironment(repoRoot, context.commitSha),
  };
  assertReleaseCheckout(repoRoot, context.commitSha, "artifact binding");
  await writeNewJson(metadataPath, metadata);

  if (githubOutput) {
    await appendGitHubOutputs(githubOutput, {
      installer_name: metadata.artifact.fileName,
      installer_path: installerPath,
      installer_sha256: sha256,
      metadata_path: path.resolve(metadataPath),
      release_sha: context.commitSha,
      release_tag: context.releaseTag,
    });
  }

  return { installerPath, metadata };
}
