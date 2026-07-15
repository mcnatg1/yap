import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import {
  bindReleaseArtifact,
  prepareReleaseContext,
  validateReleaseCoordinates,
} from "../release-artifact.mjs";
import * as releaseArtifactModule from "../release-artifact.mjs";
import { createReleaseGitFixture } from "./release-git-fixture.mjs";

test("release artifact CLI reaches argument validation without an import cycle", () => {
  const result = spawnSync(
    process.execPath,
    [path.join(import.meta.dirname, "..", "release-artifact.mjs")],
    { encoding: "utf8" },
  );

  assert.equal(result.status, 1);
  assert.match(result.stderr, /Unknown release-artifact mode: <missing>/);
  assert.doesNotMatch(result.stderr, /unsettled top-level await/i);
});

test("release artifact helper executes exact-ref binding and rejects ambiguous artifacts", async () => {
  const { commitSha, fixtureRoot: testRoot } = await createReleaseGitFixture();
  const contextPath = path.join(testRoot, "context.json");
  const bundleDirectory = path.join(testRoot, "bundle");
  const sealPath = path.join(testRoot, "artifact-seal.json");
  const metadataPath = path.join(testRoot, "metadata.json");
  const githubOutput = path.join(testRoot, "github-output.txt");
  const previousPowerShellEdition = process.env.YAP_RELEASE_POWERSHELL_EDITION;
  const previousPowerShellVersion = process.env.YAP_RELEASE_POWERSHELL_VERSION;
  try {
    process.env.YAP_RELEASE_POWERSHELL_EDITION = "Core";
    process.env.YAP_RELEASE_POWERSHELL_VERSION = "7.4.0";
    await writeFile(githubOutput, "", "utf8");
    await prepareReleaseContext({
      commitSha,
      contextPath,
      releaseTag: "v0.1.0",
      repoRoot: testRoot,
    });
    await mkdir(bundleDirectory);
    await writeFile(path.join(bundleDirectory, "Yap_0.1.0_x64-setup.exe"), "artifact", "utf8");
    assert.equal(typeof releaseArtifactModule.sealReleaseArtifact, "function");
    const sealed = await releaseArtifactModule.sealReleaseArtifact({
      bundleDirectory,
      contextPath,
      sealPath,
      repoRoot: testRoot,
    });
    const bound = await bindReleaseArtifact({
      bundleDirectory,
      contextPath,
      expectedInstallerSha256: sealed.seal.artifact.sha256,
      githubOutput,
      metadataPath,
      repoRoot: testRoot,
      sealPath,
    });
    assert.equal(bound.metadata.commitSha, commitSha);
    assert.equal(bound.metadata.releaseTag, "v0.1.0");
    assert.equal(bound.metadata.version, "0.1.0");
    assert.match(bound.metadata.commitSha, /^[0-9a-f]{40}$/);
    assert.match(bound.metadata.artifact.sha256, /^[0-9a-f]{64}$/);
    assert.equal(bound.metadata.buildEnvironment.tools.node, process.version);
    assert.equal(bound.metadata.buildEnvironment.tools.powershellEdition, "Core");
    assert.equal(bound.metadata.buildEnvironment.tools.powershellVersion, "7.4.0");
    assert.match(
      bound.metadata.buildEnvironment.inputsSha256["desktop/src-tauri/Cargo.lock"],
      /^[0-9a-f]{64}$/,
    );
    const committedConfig = execFileSync(
      "git",
      ["cat-file", "blob", `${commitSha}:desktop/src-tauri/tauri.conf.json`],
      { cwd: testRoot },
    );
    assert.equal(
      bound.metadata.buildEnvironment.inputsSha256["desktop/src-tauri/tauri.conf.json"],
      createHash("sha256").update(committedConfig).digest("hex"),
    );
    assert.match(await readFile(githubOutput, "utf8"), /release_sha=[0-9a-f]{40}/);

    const ambiguousBundle = path.join(testRoot, "ambiguous");
    await mkdir(ambiguousBundle);
    await writeFile(path.join(ambiguousBundle, "one-setup.exe"), "one", "utf8");
    await writeFile(path.join(ambiguousBundle, "two-setup.exe"), "two", "utf8");
    await assert.rejects(
      bindReleaseArtifact({
        bundleDirectory: ambiguousBundle,
        contextPath,
        expectedInstallerSha256: sealed.seal.artifact.sha256,
        metadataPath: path.join(testRoot, "ambiguous.json"),
        repoRoot: testRoot,
        sealPath,
      }),
      /exactly one NSIS installer/,
    );
    await assert.rejects(
      prepareReleaseContext({
        commitSha: "a".repeat(40),
        contextPath: path.join(testRoot, "mismatch.json"),
        releaseTag: "v0.1.0",
        repoRoot: testRoot,
      }),
      /does not match the verified release commit/,
    );
    await assert.rejects(
      prepareReleaseContext({
        commitSha,
        contextPath: path.join(testRoot, "wrong-version.json"),
        releaseTag: "v9.0.0",
        repoRoot: testRoot,
      }),
      /must exactly match desktop version/i,
    );
    assert.throws(() => validateReleaseCoordinates(commitSha, "--unsafe"), /begin with a dash/);
  } finally {
    if (previousPowerShellEdition === undefined) {
      delete process.env.YAP_RELEASE_POWERSHELL_EDITION;
    } else {
      process.env.YAP_RELEASE_POWERSHELL_EDITION = previousPowerShellEdition;
    }
    if (previousPowerShellVersion === undefined) {
      delete process.env.YAP_RELEASE_POWERSHELL_VERSION;
    } else {
      process.env.YAP_RELEASE_POWERSHELL_VERSION = previousPowerShellVersion;
    }
    await rm(testRoot, { recursive: true, force: true });
  }
});

test("release preparation rejects tracked worktree and index drift", async () => {
  const worktreeFixture = await createReleaseGitFixture("yap-release-worktree-drift-");
  try {
    await writeFile(
      path.join(worktreeFixture.fixtureRoot, "desktop/src/app.ts"),
      "export const fixture = false;\n",
    );
    await assert.rejects(
      prepareReleaseContext({
        commitSha: worktreeFixture.commitSha,
        contextPath: path.join(worktreeFixture.fixtureRoot, "context.json"),
        releaseTag: "v0.1.0",
        repoRoot: worktreeFixture.fixtureRoot,
      }),
      /tracked worktree/i,
    );
  } finally {
    await rm(worktreeFixture.fixtureRoot, { recursive: true, force: true });
  }

  const indexFixture = await createReleaseGitFixture("yap-release-index-drift-");
  try {
    const sourcePath = path.join(indexFixture.fixtureRoot, "desktop/src/app.ts");
    await writeFile(sourcePath, "export const fixture = false;\n");
    execFileSync("git", ["add", "desktop/src/app.ts"], { cwd: indexFixture.fixtureRoot });
    await writeFile(sourcePath, indexFixture.files["desktop/src/app.ts"]);
    await assert.rejects(
      prepareReleaseContext({
        commitSha: indexFixture.commitSha,
        contextPath: path.join(indexFixture.fixtureRoot, "context.json"),
        releaseTag: "v0.1.0",
        repoRoot: indexFixture.fixtureRoot,
      }),
      /index/i,
    );
  } finally {
    await rm(indexFixture.fixtureRoot, { recursive: true, force: true });
  }

  const untrackedFixture = await createReleaseGitFixture("yap-release-untracked-input-");
  try {
    const publicRoot = path.join(untrackedFixture.fixtureRoot, "desktop/public");
    await mkdir(publicRoot);
    await writeFile(path.join(publicRoot, "untracked-release-input.js"), "release mutation\n");
    await assert.rejects(
      prepareReleaseContext({
        commitSha: untrackedFixture.commitSha,
        contextPath: path.join(untrackedFixture.fixtureRoot, "context.json"),
        releaseTag: "v0.1.0",
        repoRoot: untrackedFixture.fixtureRoot,
      }),
      /untracked files/i,
    );
  } finally {
    await rm(untrackedFixture.fixtureRoot, { recursive: true, force: true });
  }

  const ignoredFixture = await createReleaseGitFixture("yap-release-ignored-input-");
  try {
    await writeFile(
      path.join(ignoredFixture.fixtureRoot, "desktop/.env.production"),
      "VITE_ENABLE_DEVELOPMENT_POLISH=true\n",
    );
    await assert.rejects(
      prepareReleaseContext({
        commitSha: ignoredFixture.commitSha,
        contextPath: path.join(ignoredFixture.fixtureRoot, "context.json"),
        releaseTag: "v0.1.0",
        repoRoot: ignoredFixture.fixtureRoot,
      }),
      /ignored release inputs/i,
    );
  } finally {
    await rm(ignoredFixture.fixtureRoot, { recursive: true, force: true });
  }
});

test("release binding rejects tracked drift after sealing", async () => {
  const { commitSha, fixtureRoot } = await createReleaseGitFixture("yap-release-bind-drift-");
  try {
    const contextPath = path.join(fixtureRoot, "context.json");
    const bundleDirectory = path.join(fixtureRoot, "bundle");
    const sealPath = path.join(fixtureRoot, "artifact-seal.json");
    await prepareReleaseContext({ commitSha, contextPath, releaseTag: "v0.1.0", repoRoot: fixtureRoot });
    await mkdir(bundleDirectory);
    await writeFile(path.join(bundleDirectory, "Yap_0.1.0_x64-setup.exe"), "artifact-A");
    const sealed = await releaseArtifactModule.sealReleaseArtifact({
      bundleDirectory,
      contextPath,
      sealPath,
      repoRoot: fixtureRoot,
    });
    await writeFile(path.join(fixtureRoot, "desktop/src/app.ts"), "export const fixture = false;\n");
    await assert.rejects(
      bindReleaseArtifact({
        bundleDirectory,
        contextPath,
        expectedInstallerSha256: sealed.seal.artifact.sha256,
        metadataPath: path.join(fixtureRoot, "metadata.json"),
        repoRoot: fixtureRoot,
        sealPath,
      }),
      /tracked worktree/i,
    );
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true });
  }

  const indexFixture = await createReleaseGitFixture("yap-release-bind-index-drift-");
  try {
    const contextPath = path.join(indexFixture.fixtureRoot, "context.json");
    const bundleDirectory = path.join(indexFixture.fixtureRoot, "bundle");
    const sealPath = path.join(indexFixture.fixtureRoot, "artifact-seal.json");
    await prepareReleaseContext({
      commitSha: indexFixture.commitSha,
      contextPath,
      releaseTag: "v0.1.0",
      repoRoot: indexFixture.fixtureRoot,
    });
    await mkdir(bundleDirectory);
    await writeFile(path.join(bundleDirectory, "Yap_0.1.0_x64-setup.exe"), "artifact-A");
    const sealed = await releaseArtifactModule.sealReleaseArtifact({
      bundleDirectory,
      contextPath,
      sealPath,
      repoRoot: indexFixture.fixtureRoot,
    });
    await writeFile(
      path.join(indexFixture.fixtureRoot, "desktop/src/app.ts"),
      "export const fixture = false;\n",
    );
    execFileSync("git", ["add", "desktop/src/app.ts"], { cwd: indexFixture.fixtureRoot });
    await assert.rejects(
      bindReleaseArtifact({
        bundleDirectory,
        contextPath,
        expectedInstallerSha256: sealed.seal.artifact.sha256,
        metadataPath: path.join(indexFixture.fixtureRoot, "metadata.json"),
        repoRoot: indexFixture.fixtureRoot,
        sealPath,
      }),
      /index/i,
    );
  } finally {
    await rm(indexFixture.fixtureRoot, { recursive: true, force: true });
  }
});

test("release binding rejects post-smoke installer substitution", async () => {
  const { commitSha, fixtureRoot } = await createReleaseGitFixture("yap-release-substitution-");
  try {
    const contextPath = path.join(fixtureRoot, "context.json");
    const bundleDirectory = path.join(fixtureRoot, "bundle");
    const installerPath = path.join(bundleDirectory, "Yap_0.1.0_x64-setup.exe");
    const sealPath = path.join(fixtureRoot, "artifact-seal.json");
    await prepareReleaseContext({ commitSha, contextPath, releaseTag: "v0.1.0", repoRoot: fixtureRoot });
    await mkdir(bundleDirectory);
    await writeFile(installerPath, "artifact-A");
    const sealed = await releaseArtifactModule.sealReleaseArtifact({
      bundleDirectory,
      contextPath,
      sealPath,
      repoRoot: fixtureRoot,
    });
    await writeFile(installerPath, "artifact-B");
    const rewrittenSeal = structuredClone(sealed.seal);
    rewrittenSeal.artifact.sha256 = createHash("sha256").update("artifact-B").digest("hex");
    rewrittenSeal.artifact.sizeBytes = Buffer.byteLength("artifact-B");
    await writeFile(sealPath, `${JSON.stringify(rewrittenSeal, null, 2)}\n`);
    await assert.rejects(
      bindReleaseArtifact({
        bundleDirectory,
        contextPath,
        expectedInstallerSha256: sealed.seal.artifact.sha256,
        metadataPath: path.join(fixtureRoot, "metadata.json"),
        repoRoot: fixtureRoot,
        sealPath,
      }),
      /immutable pre-smoke SHA-256/i,
    );
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});
