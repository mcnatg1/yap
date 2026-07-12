import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
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
import { assertReviewedRevision } from "./assert-third-party-provenance.mjs";

const require = createRequire(import.meta.url);
const { parse: parseYaml } = require("yaml");
const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..");

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

function runNodeScript(relativePath, args = []) {
  return spawnSync(process.execPath, [path.join(repoRoot, relativePath), ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

function assertExactSafeCaches(workflow) {
  for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
    for (const step of job.steps ?? []) {
      if (step.uses !== "actions/cache@v6") continue;
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
  assertExactSafeCaches(ci);
  assertExactSafeCaches(smoke);
});

test("supported release path binds one selected ref to one gated NSIS artifact", async () => {
  const release = await readWorkflow(".github/workflows/release.yml");
  const dispatch = release.on.workflow_dispatch;
  const { job, steps } = workflowSteps(release, "publish-nsis");

  assert.equal(release.on.release, undefined);
  assert.equal(dispatch.inputs.ref.required, true);
  assert.equal(dispatch.inputs.tag.required, true);
  assert.equal(release.permissions.contents, "read");
  assert.equal(job.permissions.contents, "write");
  assert.equal(job["runs-on"], "windows-latest");
  const checkout = steps.find((step) => step.uses === "actions/checkout@v7");
  assert.equal(checkout.with.ref, "${{ inputs.ref }}");
  assert.equal(checkout.with["fetch-depth"], 0);

  const prepare = namedStepIndex(steps, "Resolve selected release ref");
  const contract = namedStepIndex(steps, "Verify release contract");
  const provenance = namedStepIndex(steps, "Require reviewed third-party provenance");
  const build = namedStepIndex(steps, "Build NSIS release artifact");
  const smoke = namedStepIndex(steps, "Smoke the exact NSIS release artifact");
  const bind = namedStepIndex(steps, "Bind artifact evidence to selected ref");
  const publish = namedStepIndex(steps, "Publish gated GitHub release");
  assert.ok(prepare < contract && contract < provenance && provenance < build);
  assert.ok(build < smoke && smoke < bind && bind < publish);
  assert.match(steps[provenance].run, /--require-reviewed/);
  assert.match(steps[publish].run, /gh release create/);
  assert.match(steps[publish].run, /--draft/);
  assert.match(steps[publish].run, /gh release edit/);
  assert.equal(steps[publish].env.RELEASE_SHA, "${{ steps.bind.outputs.release_sha }}");
  assertExactSafeCaches(release);
  await assert.rejects(
    access(path.join(repoRoot, ".github/workflows/prepublish-provenance.yml")),
    /ENOENT/,
  );
});

test("release artifact helper executes exact-ref binding and rejects ambiguous artifacts", async () => {
  const testRoot = await mkdtemp(path.join(os.tmpdir(), "yap-release-contract-"));
  const contextPath = path.join(testRoot, "context.json");
  const bundleDirectory = path.join(testRoot, "bundle");
  const metadataPath = path.join(testRoot, "metadata.json");
  const githubOutput = path.join(testRoot, "github-output.txt");
  try {
    await writeFile(githubOutput, "", "utf8");
    await prepareReleaseContext({
      contextPath,
      releaseTag: "v0.1.0-contract",
      repoRoot,
      selectedRef: "hardening/yap-maintainability",
    });
    await mkdir(bundleDirectory);
    await writeFile(path.join(bundleDirectory, "Yap_0.1.0_x64-setup.exe"), "artifact", "utf8");
    const bound = await bindReleaseArtifact({
      bundleDirectory,
      contextPath,
      githubOutput,
      metadataPath,
      repoRoot,
    });
    assert.equal(bound.metadata.selectedRef, "hardening/yap-maintainability");
    assert.equal(bound.metadata.releaseTag, "v0.1.0-contract");
    assert.match(bound.metadata.commitSha, /^[0-9a-f]{40}$/);
    assert.match(bound.metadata.artifact.sha256, /^[0-9a-f]{64}$/);
    assert.match(await readFile(githubOutput, "utf8"), /release_sha=[0-9a-f]{40}/);

    const ambiguousBundle = path.join(testRoot, "ambiguous");
    await mkdir(ambiguousBundle);
    await writeFile(path.join(ambiguousBundle, "one-setup.exe"), "one", "utf8");
    await writeFile(path.join(ambiguousBundle, "two-setup.exe"), "two", "utf8");
    await assert.rejects(
      bindReleaseArtifact({
        bundleDirectory: ambiguousBundle,
        contextPath,
        metadataPath: path.join(testRoot, "ambiguous.json"),
        repoRoot,
      }),
      /exactly one NSIS installer/,
    );
    assert.throws(() => validateReleaseCoordinates("main", "--unsafe"), /begin with a dash/);
  } finally {
    await rm(testRoot, { recursive: true, force: true });
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
    "ae62240a06e155ca3dd97664c34a4ffe70cafeb18bbf8c14717855b96d9d317c",
  );
  assert.equal(freeFlow.revision.evidence.localFileEvidence, "integrity-only");
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
  const prepublish = runNodeScript(
    "desktop/tests/scripts/assert-third-party-provenance.mjs",
    ["--require-reviewed"],
  );
  assert.equal(prepublish.status, 0, prepublish.stderr);

  const arbitraryRevision = structuredClone(freeFlow);
  arbitraryRevision.revision.value = "a".repeat(40);
  assert.throws(
    () => assertReviewedRevision(arbitraryRevision),
    /does not bind its evidence to the recorded revision/,
  );
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
