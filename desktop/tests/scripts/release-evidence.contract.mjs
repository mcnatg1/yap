import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  bindReleaseArtifact,
  prepareReleaseContext,
  validateReleaseCoordinates,
} from "./release-artifact.mjs";
import * as releaseArtifactModule from "./release-artifact.mjs";
import {
  assertReviewedRevision,
  verifyReviewedSourceUpstream,
} from "./assert-third-party-provenance.mjs";

const require = createRequire(import.meta.url);
const { parse: parseYaml } = require("yaml");
const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..");
const releaseActions = Object.freeze({
  cache: "actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
  checkout: "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0",
  downloadArtifact: "actions/download-artifact@37930b1c2abaa49bbe596cd826c3c89aef350131",
  setupNode: "actions/setup-node@48b55a011bda9f5d6aeb4c2d9c7362e8dae4041e",
  setupPnpm: "pnpm/action-setup@f40ffcd9367d9f12939873eb1018b921a783ffaa",
  setupRust: "dtolnay/rust-toolchain@4be7066ada62dd38de10e7b70166bc74ed198c30",
  uploadArtifact: "actions/upload-artifact@b7c566a772e6b6bfb58ed0dc250532a479d7789f",
});

async function readRepoFile(relativePath) {
  return readFile(path.join(repoRoot, relativePath), "utf8");
}

async function readWorkflow(relativePath) {
  return parseYaml(await readRepoFile(relativePath));
}

function workflowSteps(workflow, jobName) {
  const job = workflow.jobs?.[jobName];
  assert.ok(job, `workflow is missing the ${jobName} job`);
  assert.ok(Array.isArray(job.steps), `${jobName} does not define steps`);
  return { job, steps: job.steps };
}

function namedStepIndex(steps, name) {
  const index = steps.findIndex((step) => step.name === name);
  assert.notEqual(index, -1, `workflow is missing the ${name} step`);
  return index;
}

function extractNsisMacro(source, macroName) {
  const marker = `!macro ${macroName}`;
  const start = source.indexOf(marker);
  assert.notEqual(start, -1, `NSIS hooks are missing ${macroName}`);
  const bodyStart = start + marker.length;
  const end = source.indexOf("!macroend", bodyStart);
  assert.notEqual(end, -1, `NSIS hook ${macroName} is missing !macroend`);
  return source.slice(bodyStart, end);
}

function assertPatternsInOrder(source, patterns, label) {
  let offset = 0;
  for (const pattern of patterns) {
    const match = pattern.exec(source.slice(offset));
    assert.ok(match, `${label} is missing ordered structure ${pattern}`);
    offset += match.index + match[0].length;
  }
}

function runNodeScript(relativePath, args = []) {
  return spawnSync(process.execPath, [path.join(repoRoot, relativePath), ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

async function createReleaseGitFixture(prefix = "yap-release-contract-") {
  const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), prefix));
  const files = {
    ".gitattributes": "*.nsh text\n",
    ".gitignore": ".env\n.env.*\n*.env\n/context.json\n/artifact-seal.json\n/metadata*.json\n/ambiguous.json\n/github-output.txt\n/bundle/\n/ambiguous/\n",
    "THIRD_PARTY_NOTICES.md": "fixture notices\n",
    "THIRD_PARTY_PROVENANCE.json": "{}\n",
    "desktop/package.json": `${JSON.stringify({ version: "0.1.0" })}\n`,
    "desktop/pnpm-lock.yaml": "lockfileVersion: '9.0'\n",
    "desktop/src/app.ts": "export const fixture = true;\n",
    "desktop/src-tauri/Cargo.lock": "# fixture lock\n",
    "desktop/src-tauri/Cargo.toml": "[package]\nname = 'fixture'\nversion = '0.1.0'\n",
    "desktop/src-tauri/nsis-hooks.nsh": "fixture hook\r\n",
    "desktop/src-tauri/rust-toolchain.toml": "[toolchain]\nchannel = '1.96.0'\n",
    "desktop/src-tauri/tauri.conf.json": `${JSON.stringify({ version: "0.1.0" })}\n`,
  };
  for (const [relativePath, contents] of Object.entries(files)) {
    const absolutePath = path.join(fixtureRoot, relativePath);
    await mkdir(path.dirname(absolutePath), { recursive: true });
    await writeFile(absolutePath, contents);
  }
  execFileSync("git", ["init", "-q"], { cwd: fixtureRoot });
  execFileSync("git", ["config", "user.email", "release-fixture@example.invalid"], {
    cwd: fixtureRoot,
  });
  execFileSync("git", ["config", "user.name", "Release Fixture"], { cwd: fixtureRoot });
  execFileSync("git", ["config", "core.autocrlf", "false"], { cwd: fixtureRoot });
  execFileSync("git", ["add", "."], { cwd: fixtureRoot });
  execFileSync("git", ["commit", "-q", "-m", "release fixture"], { cwd: fixtureRoot });
  const commitSha = execFileSync("git", ["rev-parse", "HEAD^{commit}"], {
    cwd: fixtureRoot,
    encoding: "utf8",
  }).trim();
  assert.equal(
    execFileSync("git", ["status", "--porcelain"], { cwd: fixtureRoot, encoding: "utf8" }).trim(),
    "",
  );
  return { commitSha, files, fixtureRoot };
}

function assertExactSafeCaches(workflow) {
  for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
    for (const step of job.steps ?? []) {
      if (!String(step.uses ?? "").startsWith("actions/cache@")) continue;
      assert.equal(
        step.with?.["restore-keys"],
        undefined,
        `${jobName} cache must not use a broad restore prefix`,
      );
      assert.match(String(step.with?.key ?? ""), /hashFiles\(/, `${jobName} cache key is not exact`);
      const cachePaths = String(step.with?.path ?? "")
        .split(/\r?\n/)
        .map((value) => value.trim())
        .filter(Boolean);
      assert.ok(cachePaths.length > 0, `${jobName} cache has no paths`);
      for (const cachePath of cachePaths) {
        assert.doesNotMatch(
          cachePath.replaceAll("\\", "/"),
          /(^|\/)target(\/|$)|(^|\/)bundle(\/|$)|(^|\/)dist(\/|$)/i,
          `${jobName} cache includes build or bundle output: ${cachePath}`,
        );
      }
    }
  }
}

test("release contract has an explicit package command outside Vitest discovery", async () => {
  const packageJson = JSON.parse(await readRepoFile("desktop/package.json"));
  assert.equal(
    packageJson.scripts["test:release-contract"],
    "pnpm check:node && node --test ./tests/scripts/release-evidence.contract.mjs",
  );
  assert.doesNotMatch(packageJson.scripts["test:release-contract"], /\.test\.[cm]?[jt]s/);
  await assert.rejects(
    access(path.join(repoRoot, "desktop/tests/scripts/release-evidence.test.mjs")),
    /ENOENT/,
  );
});

test("required native WDIO executes every deterministic spec with Mocha runtime guards", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const { job, steps } = workflowSteps(ci, "native-wdio");
  const { config } = await import("../wdio.required.conf.ts");
  const { config: hardwareConfig } = await import("../wdio.hardware.conf.ts");

  assert.deepEqual(
    config.specs.map((spec) => path.basename(spec)),
    ["smoke.spec.js", "live-overlay.spec.js", "tray-actions.spec.js"],
  );
  assert.equal(config.bail, 1);
  assert.notEqual(config.logLevel, "trace");
  assert.equal(config.mochaOpts.forbidOnly, true);
  assert.equal(config.mochaOpts.forbidPending, true);
  assert.deepEqual(
    hardwareConfig.specs.map((spec) => path.basename(spec)),
    ["live-overlay.hardware.spec.js"],
  );
  assert.equal(hardwareConfig.mochaOpts.forbidOnly, true);
  assert.equal(job["runs-on"], "windows-latest");
  assert.equal(job.env.RUST_TARGET, "x86_64-pc-windows-msvc");
  assert.ok(steps.some((step) => step.run === "pnpm test:desktop:build"));
  assert.ok(
    steps.some((step) => String(step.run ?? "").includes("wdio.required.conf.ts")),
  );
  assert.ok(
    steps.some(
      (step) => step.uses === "actions/upload-artifact@v6" && step.if === "failure()",
    ),
  );
  await assert.rejects(
    access(path.join(repoRoot, "desktop/tests/wdio/required-spec-policy.mjs")),
    /ENOENT/,
  );
});

test("CI and smoke workflows use exact dependency caches and the explicit contract command", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const smoke = await readWorkflow(".github/workflows/nsis-smoke.yml");
  const frontendSteps = workflowSteps(ci, "frontend").steps;
  const smokeSteps = workflowSteps(smoke, "nsis-bundle-smoke").steps;

  assert.ok(frontendSteps.some((step) => step.run === "pnpm test:release-contract"));
  assert.ok(smokeSteps.some((step) => step.run === "pnpm test:release-contract"));
  assert.equal(smoke.on.workflow_dispatch, null);
  assert.ok(smoke.on.schedule);
  assert.equal(smoke.on.release, undefined);
  const smokeUses = smokeSteps.filter((step) => step.uses).map((step) => step.uses);
  for (const action of smokeUses) assert.match(action, /@[0-9a-f]{40}$/);
  assertExactSafeCaches(ci);
  assertExactSafeCaches(smoke);
});

test("supported release path binds a default-branch commit to a read-only build and draft stage", async () => {
  const release = await readWorkflow(".github/workflows/release.yml");
  const dispatch = release.on.workflow_dispatch;
  const { job: resolveJob, steps: resolveSteps } = workflowSteps(release, "resolve-release");
  const { job: buildJob, steps: buildSteps } = workflowSteps(release, "build-nsis");
  const { job: publishJob, steps: publishSteps } = workflowSteps(release, "publish-nsis");

  assert.equal(release.on.release, undefined);
  assert.equal(dispatch.inputs.commit_sha.required, true);
  assert.equal(dispatch.inputs.ref, undefined);
  assert.equal(dispatch.inputs.tag.required, true);
  assert.equal(release.permissions.contents, "read");
  assert.equal(resolveJob.permissions.contents, "read");
  assert.equal(buildJob.permissions.contents, "read");
  assert.equal(publishJob.permissions.actions, "read");
  assert.equal(publishJob.permissions.contents, "write");
  assert.equal(publishJob.environment, "production-release");
  assert.deepEqual(publishJob.needs, ["resolve-release", "build-nsis"]);

  const resolveCheckout = resolveSteps.find((step) => step.uses === releaseActions.checkout);
  assert.equal(resolveCheckout.with.ref, "${{ github.event.repository.default_branch }}");
  assert.equal(resolveCheckout.with["fetch-depth"], 0);
  assert.equal(resolveCheckout.with["persist-credentials"], false);
  const resolve = resolveSteps.find((step) => step.name === "Resolve immutable commit from default branch");
  assert.match(resolve.run, /\^\[0-9a-fA-F\]\{40\}\$/);
  assert.match(resolve.run, /merge-base --is-ancestor/);
  assert.doesNotMatch(resolve.run, /git fetch|refs\/remotes\/origin/);

  const buildCheckout = buildSteps.find((step) => step.uses === releaseActions.checkout);
  assert.equal(buildCheckout.with.ref, "${{ needs.resolve-release.outputs.commit_sha }}");
  assert.equal(buildCheckout.with["persist-credentials"], false);
  const prepare = namedStepIndex(buildSteps, "Prepare immutable release context");
  const contract = namedStepIndex(buildSteps, "Verify release contract");
  const provenance = namedStepIndex(buildSteps, "Require externally verified third-party provenance");
  const build = namedStepIndex(buildSteps, "Build NSIS release artifact");
  const seal = namedStepIndex(buildSteps, "Seal exact NSIS release artifact");
  const smoke = namedStepIndex(buildSteps, "Smoke the exact NSIS release artifact");
  const captureEnvironment = namedStepIndex(buildSteps, "Capture release build environment");
  const bind = namedStepIndex(buildSteps, "Bind artifact evidence to immutable commit");
  const upload = namedStepIndex(buildSteps, "Upload immutable release payload");
  assert.ok(prepare < contract && contract < provenance && provenance < build);
  assert.ok(
    build < seal
    && seal < smoke
    && smoke < captureEnvironment
    && captureEnvironment < bind
    && bind < upload,
  );
  assert.match(buildSteps[provenance].run, /--require-reviewed/);
  assert.match(buildSteps[provenance].run, /--verify-upstream/);
  assert.match(buildSteps[seal].run, /release-artifact\.mjs seal/);
  assert.match(buildSteps[seal].run, /--seal-path/);
  assert.equal(
    buildSteps[smoke].env.SEALED_INSTALLER_SHA256,
    "${{ steps.seal.outputs.installer_sha256 }}",
  );
  assert.match(buildSteps[smoke].run, /-ExpectedInstallerSha256 \$env:SEALED_INSTALLER_SHA256/);
  assert.match(buildSteps[captureEnvironment].run, /Get-TauriNsisToolPaths/);
  assert.match(buildSteps[captureEnvironment].run, /LauncherPath \/VERSION/);
  assert.match(buildSteps[captureEnvironment].run, /CompilerPath \/VERSION/);
  assert.match(buildSteps[captureEnvironment].run, /NSIS_LAUNCHER_SHA256/);
  assert.match(buildSteps[captureEnvironment].run, /NSIS_COMPILER_SHA256/);
  assert.match(buildSteps[bind].run, /--seal-path/);
  assert.equal(
    buildSteps[bind].env.SEALED_INSTALLER_SHA256,
    "${{ steps.seal.outputs.installer_sha256 }}",
  );
  assert.match(buildSteps[bind].run, /--expected-installer-sha256/);
  assert.match(buildSteps[upload].with.name, /needs\.resolve-release\.outputs\.commit_sha/);

  assert.equal(publishSteps.some((step) => String(step.uses ?? "").startsWith("actions/checkout@")), false);
  const releaseUses = [...resolveSteps, ...buildSteps, ...publishSteps]
    .filter((step) => step.uses)
    .map((step) => step.uses);
  assert.deepEqual(new Set(releaseUses), new Set(Object.values(releaseActions)));
  for (const action of releaseUses) assert.match(action, /@[0-9a-f]{40}$/);
  const verify = publishSteps.find((step) => step.name === "Verify downloaded payload binding");
  assert.match(verify.run, /Get-FileHash/);
  assert.match(verify.run, /VERIFIED_SHA/);
  assert.match(verify.run, /RELEASE_TAG/);
  assert.match(verify.run, /metadata\.version/);
  assert.match(verify.run, /buildEnvironment\.runner\.imageOs/);
  assert.match(verify.run, /"rustcVv"/);
  assert.match(verify.run, /nsisLauncherSha256/);
  assert.match(verify.run, /nsisCompilerSha256/);
  assert.match(verify.run, /buildEnvironment\.inputsSha256/);
  const environmentPolicy = publishSteps.find(
    (step) => step.name === "Verify production release environment policy",
  );
  assert.match(environmentPolicy.run, /deployment-branch-policies/);
  assert.doesNotMatch(environmentPolicy.run, /required_reviewers/);
  const tagBinding = publishSteps.find(
    (step) => step.name === "Bind release tag to verified commit",
  );
  assert.match(tagBinding.run, /git\/refs/);
  assert.match(tagBinding.run, /commits\/\$encodedTag/);
  assert.match(tagBinding.run, /RELEASE_SHA/);
  const policy = publishSteps.find((step) => step.name === "Record enforcement boundary");
  assert.match(policy.run, /publish the draft manually/i);
  const publish = publishSteps.find((step) => step.name === "Stage verified GitHub draft release");
  assert.match(publish.run, /gh release create/);
  assert.match(publish.run, /--draft/);
  assert.match(publish.run, /--verify-tag/);
  assert.doesNotMatch(publish.run, /gh release edit|--draft=false/);
  assert.equal(publish.env.GH_REPO, "${{ github.repository }}");
  assert.equal(publish.env.RELEASE_SHA, "${{ needs.resolve-release.outputs.commit_sha }}");
  assertExactSafeCaches(release);
  await assert.rejects(
    access(path.join(repoRoot, ".github/workflows/prepublish-provenance.yml")),
    /ENOENT/,
  );
});

test("release artifact helper executes exact-ref binding and rejects ambiguous artifacts", async () => {
  const { commitSha, fixtureRoot: testRoot } = await createReleaseGitFixture();
  const contextPath = path.join(testRoot, "context.json");
  const bundleDirectory = path.join(testRoot, "bundle");
  const sealPath = path.join(testRoot, "artifact-seal.json");
  const metadataPath = path.join(testRoot, "metadata.json");
  const githubOutput = path.join(testRoot, "github-output.txt");
  try {
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
    assert.match(
      bound.metadata.buildEnvironment.inputsSha256["desktop/src-tauri/Cargo.lock"],
      /^[0-9a-f]{64}$/,
    );
    const committedHook = execFileSync(
      "git",
      ["cat-file", "blob", `${commitSha}:desktop/src-tauri/nsis-hooks.nsh`],
      { cwd: testRoot },
    );
    assert.equal(
      bound.metadata.buildEnvironment.inputsSha256["desktop/src-tauri/nsis-hooks.nsh"],
      createHash("sha256").update(committedHook).digest("hex"),
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
    assert.equal(typeof releaseArtifactModule.sealReleaseArtifact, "function");
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
    assert.equal(typeof releaseArtifactModule.sealReleaseArtifact, "function");
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

test("provenance gate requires exact scoped review evidence and current local hashes", async () => {
  const manifest = JSON.parse(await readRepoFile("THIRD_PARTY_PROVENANCE.json"));
  const tauriConfig = JSON.parse(await readRepoFile("desktop/src-tauri/tauri.conf.json"));
  const notice = await readRepoFile("THIRD_PARTY_NOTICES.md");
  const freeFlow = manifest.sources.find(({ id }) => id === "freeflow");

  assert.equal(manifest.schemaVersion, 2);
  assert.equal(freeFlow.repository, "https://github.com/zachlatta/freeflow");
  assert.equal(freeFlow.license, "MIT");
  assert.equal(freeFlow.revision.status, "reviewed");
  assert.equal(freeFlow.revision.value, "7427ca982c19746770f5357ced16e993f2eb27fd");
  assert.equal(
    freeFlow.revision.evidence.licenseSha256,
    "121e01b10b43ece3c10ce3eaf5db22915326aad843c3a271a834660096467add",
  );
  assert.equal(freeFlow.revision.evidence.localFileEvidence, "integrity-only");
  assert.equal(freeFlow.revision.evidence.upstreamFiles.length, 2);
  assert.equal(freeFlow.notice, "THIRD_PARTY_NOTICES.md");
  assert.equal(freeFlow.files.length, 2);
  for (const file of freeFlow.files) {
    assert.match(file.sha256, /^[0-9a-f]{64}$/);
    assert.equal(typeof file.path, "string");
  }
  assert.match(notice, /^## FreeFlow$/m);
  assert.match(notice, /^MIT License$/m);
  assert.equal(
    tauriConfig.bundle.resources?.["../../THIRD_PARTY_NOTICES.md"],
    "THIRD_PARTY_NOTICES.md",
  );
  assert.equal(
    tauriConfig.bundle.resources?.["../../THIRD_PARTY_PROVENANCE.json"],
    "THIRD_PARTY_PROVENANCE.json",
  );

  const integrity = runNodeScript("desktop/tests/scripts/assert-third-party-provenance.mjs");
  assert.equal(integrity.status, 0, integrity.stderr);
  const arbitraryRevision = structuredClone(freeFlow);
  arbitraryRevision.revision.value = "a".repeat(40);
  assert.throws(
    () => assertReviewedRevision(arbitraryRevision),
    /does not bind its evidence to the recorded revision/,
  );

  const fixtureSource = structuredClone(freeFlow);
  const licenseBytes = Buffer.from("immutable license fixture\n", "utf8");
  const upstreamBytes = Buffer.from("immutable upstream source\n", "utf8");
  fixtureSource.revision.evidence.licenseSha256 = createHash("sha256")
    .update(licenseBytes)
    .digest("hex");
  fixtureSource.revision.evidence.upstreamFiles = [{
    path: "Sources/Fixture.swift",
    sha256: createHash("sha256").update(upstreamBytes).digest("hex"),
  }];
  const requested = [];
  const fetchImpl = async (url) => {
    requested.push(String(url));
    if (String(url).includes("api.github.com")) {
      return new Response(JSON.stringify({ sha: fixtureSource.revision.value }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }
    return new Response(
      String(url).endsWith("/LICENSE") ? licenseBytes : upstreamBytes,
      { status: 200 },
    );
  };
  await verifyReviewedSourceUpstream(fixtureSource, { fetchImpl, timeoutMs: 1_000 });
  assert.deepEqual(requested, [
    `https://api.github.com/repos/zachlatta/freeflow/commits/${fixtureSource.revision.value}`,
    `https://raw.githubusercontent.com/zachlatta/freeflow/${fixtureSource.revision.value}/LICENSE`,
    `https://raw.githubusercontent.com/zachlatta/freeflow/${fixtureSource.revision.value}/Sources/Fixture.swift`,
  ]);

  await assert.rejects(
    verifyReviewedSourceUpstream(fixtureSource, {
      fetchImpl: async () => {
        throw new Error("offline");
      },
      timeoutMs: 1_000,
    }),
    /upstream verification failed.*offline/i,
  );
  await assert.rejects(
    verifyReviewedSourceUpstream(fixtureSource, {
      fetchImpl: async (_url, { signal }) => new Response(new ReadableStream({
        start(controller) {
          signal.addEventListener("abort", () => controller.error(signal.reason), { once: true });
        },
      }), { status: 200 }),
      timeoutMs: 50,
    }),
    /request exceeded 50 ms/i,
  );
  await assert.rejects(
    verifyReviewedSourceUpstream(fixtureSource, {
      fetchImpl: async (url) => String(url).includes("api.github.com")
        ? new Response(JSON.stringify({ sha: "b".repeat(40) }), { status: 200 })
        : new Response(licenseBytes, { status: 200 }),
      timeoutMs: 1_000,
    }),
    /returned a different commit/i,
  );
  await assert.rejects(
    verifyReviewedSourceUpstream(fixtureSource, {
      fetchImpl: async (url) => String(url).includes("api.github.com")
        ? new Response(JSON.stringify({ sha: fixtureSource.revision.value }), { status: 200 })
        : new Response("wrong license", { status: 200 }),
      timeoutMs: 1_000,
    }),
    /license hash mismatch/i,
  );
});

test("NSIS smoke separates local-safe validation from isolated production deletion", async () => {
  const hooks = await readRepoFile("desktop/src-tauri/nsis-hooks.nsh");
  const smoke = await readRepoFile("desktop/tests/scripts/smoke-nsis.ps1");
  const smokeHelpers = await readRepoFile("desktop/tests/scripts/nsis-smoke-helpers.psm1");
  const localSmoke = await readRepoFile("desktop/tests/scripts/smoke-nsis-local.ps1");
  const testDeleteSmoke = await readRepoFile("desktop/tests/scripts/smoke-nsis-test-delete.ps1");
  const productionDeleteSmoke = await readRepoFile("desktop/tests/scripts/smoke-nsis-production-delete.ps1");
  const testConfig = JSON.parse(await readRepoFile("desktop/src-tauri/tauri.test.conf.json"));

  const preUninstall = extractNsisMacro(hooks, "NSIS_HOOK_PREUNINSTALL");
  const postUninstall = extractNsisMacro(hooks, "NSIS_HOOK_POSTUNINSTALL");
  assert.match(hooks, /!macro YAP_ABORT_DELETE_WITH_ROLLBACK/);
  assert.match(hooks, /Function un\.YapValidateDeleteEntry/);
  assert.match(hooks, /\$\{un\.GetFileAttributes\} "\$R9" "REPARSE_POINT"/);
  assert.doesNotMatch(preUninstall, /^\s*(?:Rename|RMDir)\b/m);
  assert.match(preUninstall, /YAP_VALIDATE_DELETE_TREE "\$APPDATA\\\$\{BUNDLEID\}"/);
  assert.match(preUninstall, /YAP_VALIDATE_DELETE_TREE "\$LOCALAPPDATA\\\$\{BUNDLEID\}"/);
  assert.doesNotMatch(
    postUninstall,
    /^\s*RMDir \/r "\$LOCALAPPDATA\\\$\{PRODUCTNAME\}"\s*$/m,
  );
  assertPatternsInOrder(
    preUninstall,
    [
      /\$CMDLINE "\/DELETEAPPDATA=" \$YapDeleteToken/,
      /RUNNER_ENVIRONMENT/,
      /FileOpen[^\n]+\.yap-destructive-uninstall-test/,
      /\$R2 != \$YapDeleteToken/,
      /StrCpy \$YapAutomatedDelete "1"/,
      /StrCpy \$DeleteAppDataCheckboxState 1/,
    ],
    "NSIS pre-uninstall authorization",
  );
  assertPatternsInOrder(
    postUninstall,
    [
      /\$DeleteAppDataCheckboxState == 1/,
      /\$YapAutomatedDelete == "1"/,
      /FileOpen[^\n]+\.yap-destructive-uninstall-test/,
      /\$R2 != \$YapDeleteToken/,
      /\$\{FileExists\}[^\n]+\.delete-quarantine/,
      /Rename[^\n]+\.delete-quarantine/,
      /\$\{un\.GetFileAttributes\}[^\n]+\.delete-quarantine[^\n]+"REPARSE_POINT"/,
      /\$R6 == "1"/,
      /\$\{un\.Locate\}[^\n]+\.delete-quarantine[^\n]+\/G=1/,
      /\$YapDeleteValidationFailure != ""/,
      /FileOpen[^\n]+\.delete-quarantine\\\.yap-destructive-uninstall-test/,
      /\$R2 != \$YapDeleteToken/,
      /RMDir \/r[^\n]+\.delete-quarantine/,
      /\$\{FileExists\}[^\n]+\.delete-quarantine/,
    ],
    "NSIS post-uninstall authenticated quarantine deletion",
  );
  assert.match(smoke, /defaultPreservedData/);
  assert.match(smoke, /explicitDeletion/);
  assert.match(smoke, /GITHUB_ACTIONS/);
  assert.match(smoke, /RUNNER_ENVIRONMENT/);
  assert.match(smoke, /github-hosted/);
  assert.match(smoke, /Windows Sandbox/);
  assert.match(smoke, /Yap\.Test/);
  assert.match(smoke, /YAP_APP_DATA_DIR/);
  assert.match(smoke, /IsolatedProductionDelete/);
  assert.match(smoke, /VersionInfo\.ProductName/);
  assert.match(smoke, /ExpectedInstallerSha256/);
  assert.match(smoke, /artifactIntegrity/);
  assert.match(smoke, /beforeSha256/);
  assert.match(smoke, /afterSha256/);
  assert.doesNotMatch(smoke, /AllowProfileMutation/);
  assert.match(smoke, /dataMarkerPaths/);
  assert.match(smoke, /deleteQuarantine/);
  assert.match(smoke, /Remove-OwnedDeleteQuarantine/);
  assert.match(smoke, /Delete-quarantine cleanup refuses data from another test run/);
  assert.match(smoke, /Get-Content[^\n]+markerPath[^\n]+-Raw/);
  assert.match(smoke, /-ArgumentList @\("\/S"\)/);
  assert.match(smoke, /-ArgumentList @\("\/S", "\/DELETEAPPDATA=\$runToken"\)/);
  assert.match(smoke, /Import-Module[^\n]+nsis-smoke-helpers\.psm1/);
  assert.match(smoke, /Enter-SmokeRunLock/);
  assert.match(smokeHelpers, /System\.Threading\.Mutex/);
  assert.match(localSmoke, /-Mode LocalSafe/);
  assert.match(localSmoke, /-ExpectedInstallerSha256 \$ExpectedInstallerSha256/);
  assert.match(testDeleteSmoke, /-Mode TestIdentityDelete/);
  assert.match(testDeleteSmoke, /-ExpectedInstallerSha256 \$ExpectedInstallerSha256/);
  assert.match(productionDeleteSmoke, /-Mode IsolatedProductionDelete/);
  assert.match(productionDeleteSmoke, /-ExpectedInstallerSha256 \$ExpectedInstallerSha256/);
  assert.equal(testConfig.productName, "Yap.Test");
  assert.equal(testConfig.identifier, "com.mcnatg1.yap.test");
  assert.equal(testConfig.mainBinaryName, "yap-test");
});

test("NSIS helper behavior passes its focused PowerShell suite", () => {
  const result = spawnSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      path.join(repoRoot, "desktop/tests/scripts/nsis-smoke-helpers.test.ps1"),
    ],
    { cwd: repoRoot, encoding: "utf8", timeout: 60_000 },
  );
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
});
