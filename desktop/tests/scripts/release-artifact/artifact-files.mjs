import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { appendFile, mkdir, readdir, stat, writeFile } from "node:fs/promises";
import path from "node:path";

import {
  assertSafeText,
  installerNameMatchesVersion,
} from "./release-state.mjs";

export async function resolveInstaller(bundleDirectory, version) {
  const resolvedBundleDirectory = path.resolve(bundleDirectory);
  const entries = await readdir(resolvedBundleDirectory, { withFileTypes: true });
  const installers = entries.filter(
    (entry) => entry.isFile() && /-setup\.exe$/i.test(entry.name),
  );
  if (installers.length !== 1) {
    throw new Error(
      `Expected exactly one NSIS installer in ${bundleDirectory}; found ${installers.length}.`,
    );
  }

  const installerPath = path.resolve(resolvedBundleDirectory, installers[0].name);
  const relativeInstaller = path.relative(resolvedBundleDirectory, installerPath);
  if (
    relativeInstaller === "" ||
    relativeInstaller === ".." ||
    relativeInstaller.startsWith(`..${path.sep}`) ||
    path.isAbsolute(relativeInstaller)
  ) {
    throw new Error("Resolved NSIS installer escaped its bundle directory.");
  }

  const installerStat = await stat(installerPath);
  if (!installerStat.isFile()) throw new Error("Resolved NSIS installer is not a regular file.");
  if (!installerNameMatchesVersion(path.basename(installerPath), version)) {
    throw new Error(`NSIS installer filename does not match release version ${version}.`);
  }
  return { installerPath, installerStat };
}

export function assertArtifactSeal(seal, context) {
  if (seal?.schemaVersion !== 1) throw new Error("Unsupported release artifact seal schema.");
  if (
    seal.commitSha !== context.commitSha ||
    seal.releaseTag !== context.releaseTag ||
    seal.version !== context.version
  ) {
    throw new Error("Release artifact seal coordinates do not match the immutable context.");
  }
  if (
    typeof seal.artifact?.fileName !== "string" ||
    !installerNameMatchesVersion(seal.artifact.fileName, context.version)
  ) {
    throw new Error("Release artifact seal contains an invalid installer name.");
  }
  assertSha256(seal.artifact.sha256, "release artifact seal");
  if (!Number.isSafeInteger(seal.artifact.sizeBytes) || seal.artifact.sizeBytes < 0) {
    throw new Error("Release artifact seal contains an invalid installer size.");
  }
}

export function assertSha256(value, label) {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) {
    throw new Error(`${label} SHA-256 is invalid.`);
  }
}

export async function appendGitHubOutputs(githubOutput, outputs) {
  for (const [key, value] of Object.entries(outputs)) {
    assertSafeText(value, `GitHub output ${key}`, 32_768);
    await appendFile(githubOutput, `${key}=${value}\n`, "utf8");
  }
}

export async function hashFile(filePath) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(filePath)) hash.update(chunk);
  return hash.digest("hex");
}

export async function writeNewJson(filePath, value) {
  const resolved = path.resolve(filePath);
  await mkdir(path.dirname(resolved), { recursive: true });
  await writeFile(resolved, `${JSON.stringify(value, null, 2)}\n`, {
    encoding: "utf8",
    flag: "wx",
  });
}
