import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import path from "node:path";

const releaseInputPaths = Object.freeze([
  "THIRD_PARTY_NOTICES.md",
  "THIRD_PARTY_PROVENANCE.json",
  "desktop/package.json",
  "desktop/pnpm-lock.yaml",
  "desktop/src-tauri/Cargo.lock",
  "desktop/src-tauri/Cargo.toml",
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

export function assertReleaseCheckout(repoRoot, commitSha, phase) {
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

export function assertReleaseContext(context) {
  if (context?.schemaVersion !== 3) throw new Error("Unsupported release context schema.");
  validateReleaseCoordinates(context.commitSha, context.releaseTag);
  validateReleaseVersion(context.releaseTag, context.version);
}

export function readReleaseVersion(repoRoot, commitSha) {
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

export function validateReleaseVersion(releaseTag, version) {
  if (typeof version !== "string" || releaseTag !== `v${version}`) {
    throw new Error(`Release tag must exactly match desktop version v${version}.`);
  }
}

export function installerNameMatchesVersion(fileName, version) {
  return fileName.includes(`_${version}_`) && /-setup\.exe$/i.test(fileName);
}

export function collectBuildEnvironment(repoRoot, commitSha) {
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

export function assertSafeText(value, label, maxLength) {
  if (typeof value !== "string" || value.length === 0 || value.length > maxLength) {
    throw new Error(`Invalid ${label}.`);
  }
  if (/[\0\r\n]/.test(value)) throw new Error(`${label} contains a control character.`);
}

function readHeadCommit(repoRoot) {
  const commitSha = execFileSync(
    "git",
    ["-C", path.resolve(repoRoot), "rev-parse", "HEAD^{commit}"],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    },
  ).trim();
  if (!/^[0-9a-f]{40}$/.test(commitSha)) {
    throw new Error(`Git returned an invalid release commit: ${commitSha}`);
  }
  return commitSha;
}

function assertNoUntrackedFiles(repoRoot, phase) {
  const result = spawnSync(
    "git",
    ["-C", path.resolve(repoRoot), "ls-files", "--others", "--exclude-standard", "-z", "--"],
    { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
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
    { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
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

function hashBytes(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}
