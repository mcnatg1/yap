import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { createReadStream } from "node:fs";
import { appendFile, mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const releaseInputPaths = Object.freeze([
  "THIRD_PARTY_NOTICES.md",
  "THIRD_PARTY_PROVENANCE.json",
  "desktop/package.json",
  "desktop/pnpm-lock.yaml",
  "desktop/src-tauri/Cargo.lock",
  "desktop/src-tauri/Cargo.toml",
  "desktop/src-tauri/nsis-hooks.nsh",
  "desktop/src-tauri/rust-toolchain.toml",
  "desktop/src-tauri/tauri.conf.json",
]);
const ignoredReleaseInputPathspecs = Object.freeze([
  "desktop/.env",
  "desktop/.env.*",
  "desktop/index.html",
  "desktop/public",
  "desktop/src",
  "desktop/tsconfig.json",
  "desktop/vite.config.ts",
  "desktop/src-tauri/build.rs",
  "desktop/src-tauri/capabilities",
  "desktop/src-tauri/icons",
  "desktop/src-tauri/nsis-hooks.nsh",
  "desktop/src-tauri/src",
  "desktop/src-tauri/Cargo.lock",
  "desktop/src-tauri/Cargo.toml",
  "desktop/src-tauri/rust-toolchain.toml",
  "desktop/src-tauri/tauri.conf.json",
]);

export function validateReleaseCoordinates(commitSha, releaseTag) {
  assertSafeText(commitSha, "release commit", 40);
  if (!/^[0-9a-f]{40}$/.test(commitSha)) {
    throw new Error("Release commit must be an immutable full lowercase SHA.");
  }
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

async function resolveInstaller(bundleDirectory, version) {
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

function assertReleaseCheckout(repoRoot, commitSha, phase) {
  const checkoutCommit = readHeadCommit(repoRoot);
  if (checkoutCommit !== commitSha) {
    throw new Error(
      `Release checkout ${checkoutCommit} does not match the verified release commit ${commitSha} during ${phase}.`,
    );
  }

  assertGitDiffClean(
    repoRoot,
    ["diff", "--no-ext-diff", "--cached", "--quiet", "--exit-code", commitSha, "--"],
    `Release index differs from verified commit ${commitSha} during ${phase}.`,
  );
  assertGitDiffClean(
    repoRoot,
    ["diff", "--no-ext-diff", "--quiet", "--exit-code", "--"],
    `Release tracked worktree differs from its index during ${phase}.`,
  );
  assertNoUntrackedFiles(repoRoot, phase);
  assertNoIgnoredReleaseInputs(repoRoot, phase);
}

function assertNoUntrackedFiles(repoRoot, phase) {
  const result = spawnSync(
    "git",
    ["-C", path.resolve(repoRoot), "ls-files", "--others", "--exclude-standard", "-z", "--"],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  );
  if (result.error) {
    throw new Error(`Failed to inspect untracked release inputs: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Git could not inspect untracked release inputs (exit ${result.status ?? "unknown"}): ${String(result.stderr ?? "").trim()}`,
    );
  }
  const untracked = String(result.stdout ?? "").split("\0").filter(Boolean);
  if (untracked.length > 0) {
    throw new Error(
      `Release checkout contains untracked files during ${phase}: ${untracked.slice(0, 10).join(", ")}`,
    );
  }
}

function assertNoIgnoredReleaseInputs(repoRoot, phase) {
  const result = spawnSync(
    "git",
    [
      "-C",
      path.resolve(repoRoot),
      "ls-files",
      "--others",
      "--ignored",
      "--exclude-standard",
      "-z",
      "--",
      ...ignoredReleaseInputPathspecs,
    ],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  );
  if (result.error) {
    throw new Error(`Failed to inspect ignored release inputs: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Git could not inspect ignored release inputs (exit ${result.status ?? "unknown"}): ${String(result.stderr ?? "").trim()}`,
    );
  }
  const ignored = String(result.stdout ?? "").split("\0").filter(Boolean);
  if (ignored.length > 0) {
    throw new Error(
      `Release checkout contains ignored release inputs during ${phase}: ${ignored.slice(0, 10).join(", ")}`,
    );
  }
}

function assertGitDiffClean(repoRoot, gitArgs, driftMessage) {
  const result = spawnSync("git", ["-C", path.resolve(repoRoot), ...gitArgs], {
    encoding: "utf8",
    stdio: ["ignore", "ignore", "pipe"],
    windowsHide: true,
  });
  if (result.error) {
    throw new Error(`Failed to inspect release checkout: ${result.error.message}`);
  }
  if (result.status === 0) return;
  if (result.status === 1) throw new Error(driftMessage);
  throw new Error(
    `Git could not inspect the release checkout (exit ${result.status ?? "unknown"}): ${String(result.stderr ?? "").trim()}`,
  );
}

function readGitBlob(repoRoot, commitSha, relativePath) {
  const gitPath = relativePath.replaceAll("\\", "/");
  try {
    return execFileSync(
      "git",
      ["-C", path.resolve(repoRoot), "cat-file", "blob", `${commitSha}:${gitPath}`],
      {
        encoding: null,
        maxBuffer: 64 * 1024 * 1024,
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      },
    );
  } catch {
    throw new Error(`Verified release commit is missing required input ${gitPath}.`);
  }
}

function assertReleaseContext(context) {
  if (context?.schemaVersion !== 3) throw new Error("Unsupported release context schema.");
  validateReleaseCoordinates(context.commitSha, context.releaseTag);
  validateReleaseVersion(context.releaseTag, context.version);
}

function assertArtifactSeal(seal, context) {
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
  if (typeof seal.artifact.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(seal.artifact.sha256)) {
    throw new Error("Release artifact seal contains an invalid SHA-256.");
  }
  if (!Number.isSafeInteger(seal.artifact.sizeBytes) || seal.artifact.sizeBytes < 0) {
    throw new Error("Release artifact seal contains an invalid installer size.");
  }
}

function assertSha256(value, label) {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) {
    throw new Error(`${label} SHA-256 is invalid.`);
  }
}

function readReleaseVersion(repoRoot, commitSha) {
  const packageManifest = JSON.parse(
    readGitBlob(repoRoot, commitSha, "desktop/package.json").toString("utf8"),
  );
  const tauriManifest = JSON.parse(
    readGitBlob(repoRoot, commitSha, "desktop/src-tauri/tauri.conf.json").toString("utf8"),
  );
  const version = packageManifest?.version;
  if (typeof version !== "string" || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error("Desktop package has an invalid release version.");
  }
  if (tauriManifest?.version !== version) {
    throw new Error("Desktop package and Tauri bundle versions do not match.");
  }
  return version;
}

function validateReleaseVersion(releaseTag, version) {
  if (typeof version !== "string" || releaseTag !== `v${version}`) {
    throw new Error(`Release tag must exactly match desktop version v${version}.`);
  }
}

function installerNameMatchesVersion(fileName, version) {
  return fileName.includes(`_${version}_`) && /-setup\.exe$/i.test(fileName);
}

function collectBuildEnvironment(repoRoot, commitSha) {
  const inputSha256 = {};
  for (const relativePath of releaseInputPaths) {
    inputSha256[relativePath] = hashBytes(readGitBlob(repoRoot, commitSha, relativePath));
  }
  return {
    inputsSha256: inputSha256,
    runner: {
      imageOs: process.env.ImageOS ?? null,
      imageVersion: process.env.ImageVersion ?? null,
    },
    tools: {
      node: process.version,
      nsis: process.env.YAP_RELEASE_NSIS_VERSION ?? null,
      nsisCompilerSha256: process.env.YAP_RELEASE_NSIS_COMPILER_SHA256 ?? null,
      nsisLauncherSha256: process.env.YAP_RELEASE_NSIS_LAUNCHER_SHA256 ?? null,
      pnpm: process.env.YAP_RELEASE_PNPM_VERSION ?? null,
      powershellEdition: process.env.YAP_RELEASE_POWERSHELL_EDITION ?? null,
      powershellVersion: process.env.YAP_RELEASE_POWERSHELL_VERSION ?? null,
      rustcVv: process.env.YAP_RELEASE_RUSTC_VV ?? null,
      tauriCli: process.env.YAP_RELEASE_TAURI_VERSION ?? null,
    },
  };
}

async function appendGitHubOutputs(githubOutput, outputs) {
  for (const [key, value] of Object.entries(outputs)) {
    assertSafeText(value, `GitHub output ${key}`, 32_768);
    await appendFile(githubOutput, `${key}=${value}\n`, "utf8");
  }
}

function hashBytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
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

const entryPoint = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : "";
if (entryPoint === import.meta.url) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
