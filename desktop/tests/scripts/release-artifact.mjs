import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { createReadStream } from "node:fs";
import { appendFile, mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

export function validateReleaseCoordinates(selectedRef, releaseTag) {
  assertSafeText(selectedRef, "selected ref", 256);
  assertSafeText(releaseTag, "release tag", 128);
  if (releaseTag.startsWith("-")) throw new Error("Release tag must not begin with a dash.");
  try {
    execFileSync("git", ["check-ref-format", `refs/tags/${releaseTag}`], {
      stdio: "pipe",
      windowsHide: true,
    });
  } catch {
    throw new Error(`Invalid Git release tag: ${releaseTag}`);
  }
}

export async function prepareReleaseContext({
  contextPath,
  releaseTag,
  repoRoot,
  selectedRef,
}) {
  validateReleaseCoordinates(selectedRef, releaseTag);
  const commitSha = readHeadCommit(repoRoot);
  const context = {
    schemaVersion: 1,
    selectedRef,
    releaseTag,
    commitSha,
  };
  await writeNewJson(contextPath, context);
  return context;
}

export async function bindReleaseArtifact({
  bundleDirectory,
  contextPath,
  githubOutput,
  metadataPath,
  repoRoot,
}) {
  const context = JSON.parse(await readFile(contextPath, "utf8"));
  assertReleaseContext(context);
  const currentCommit = readHeadCommit(repoRoot);
  if (currentCommit !== context.commitSha) {
    throw new Error(
      `Release checkout moved after provenance verification: ${context.commitSha} -> ${currentCommit}.`,
    );
  }

  const entries = await readdir(bundleDirectory, { withFileTypes: true });
  const installers = entries.filter(
    (entry) => entry.isFile() && /-setup\.exe$/i.test(entry.name),
  );
  if (installers.length !== 1) {
    throw new Error(
      `Expected exactly one NSIS installer in ${bundleDirectory}; found ${installers.length}.`,
    );
  }

  const installerPath = path.resolve(bundleDirectory, installers[0].name);
  const relativeInstaller = path.relative(path.resolve(bundleDirectory), installerPath);
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
  const sha256 = await hashFile(installerPath);
  const metadata = {
    ...context,
    artifact: {
      fileName: path.basename(installerPath),
      sha256,
      sizeBytes: installerStat.size,
    },
  };
  await writeNewJson(metadataPath, metadata);

  if (githubOutput) {
    const outputs = {
      installer_name: metadata.artifact.fileName,
      installer_path: installerPath,
      installer_sha256: sha256,
      metadata_path: path.resolve(metadataPath),
      release_sha: context.commitSha,
      release_tag: context.releaseTag,
    };
    for (const [key, value] of Object.entries(outputs)) {
      assertSafeText(value, `GitHub output ${key}`, 32_768);
      await appendFile(githubOutput, `${key}=${value}\n`, "utf8");
    }
  }

  return { installerPath, metadata };
}

function readHeadCommit(repoRoot) {
  const commitSha = execFileSync("git", ["-C", path.resolve(repoRoot), "rev-parse", "HEAD^{commit}"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  }).trim();
  if (!/^[0-9a-f]{40}$/.test(commitSha)) {
    throw new Error(`Git returned an invalid release commit: ${commitSha}`);
  }
  return commitSha;
}

function assertReleaseContext(context) {
  if (context?.schemaVersion !== 1) throw new Error("Unsupported release context schema.");
  validateReleaseCoordinates(context.selectedRef, context.releaseTag);
  if (!/^[0-9a-f]{40}$/.test(context.commitSha)) {
    throw new Error("Release context does not contain a full commit SHA.");
  }
}

async function hashFile(filePath) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(filePath)) hash.update(chunk);
  return hash.digest("hex");
}

async function writeNewJson(filePath, value) {
  const resolved = path.resolve(filePath);
  await mkdir(path.dirname(resolved), { recursive: true });
  await writeFile(resolved, `${JSON.stringify(value, null, 2)}\n`, {
    encoding: "utf8",
    flag: "wx",
  });
}

function assertSafeText(value, label, maxLength) {
  if (typeof value !== "string" || value.length === 0 || value.length > maxLength) {
    throw new Error(`Invalid ${label}.`);
  }
  if (/[\0\r\n]/.test(value)) throw new Error(`${label} contains a control character.`);
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

async function main(args) {
  const { mode, values } = parseArguments(args);
  const repoRoot = path.resolve(values.get("--repo-root") ?? process.cwd());
  if (mode === "prepare") {
    const context = await prepareReleaseContext({
      contextPath: required(values, "--context-path"),
      releaseTag: required(values, "--release-tag"),
      repoRoot,
      selectedRef: required(values, "--selected-ref"),
    });
    console.log(JSON.stringify(context));
    return;
  }
  if (mode === "bind") {
    const result = await bindReleaseArtifact({
      bundleDirectory: required(values, "--bundle-directory"),
      contextPath: required(values, "--context-path"),
      githubOutput: values.get("--github-output"),
      metadataPath: required(values, "--metadata-path"),
      repoRoot,
    });
    console.log(JSON.stringify(result.metadata));
    return;
  }
  throw new Error(`Unknown release-artifact mode: ${mode ?? "<missing>"}`);
}

const entryPoint = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : "";
if (entryPoint === import.meta.url) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
