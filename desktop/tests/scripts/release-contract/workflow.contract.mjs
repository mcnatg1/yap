import assert from "node:assert/strict";
import { access } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { assertReviewedActionPins } from "./action-policy.mjs";
import {
  discoveredWorkflowPaths,
  normalizedRunBody,
  readRepoFile,
  readWorkflow,
  repoRoot,
  workflowSteps,
} from "./workflow-access.mjs";
import {
  pnpmStoreBindingScriptPath,
  reviewedActions,
  reviewedCargoAuditRun,
  reviewedWindowsGraphBoundaryRun,
  workflowPaths,
} from "./workflow-policy.mjs";

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
  const { config } = await import("../../wdio.required.conf.ts");
  const { config: hardwareConfig } = await import("../../wdio.hardware.conf.ts");

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
  assert.ok(steps.some((step) => String(step.run ?? "").includes("wdio.required.conf.ts")));
  assert.ok(
    steps.some(
      (step) => step.uses === reviewedActions.uploadArtifact && step.if === "failure()",
    ),
    "native WDIO failure artifacts must use the reviewed upload-artifact v7.0.1 pin",
  );
  await assert.rejects(
    access(path.join(repoRoot, "desktop/tests/wdio/required-spec-policy.mjs")),
    /ENOENT/,
  );
});

test("CI and smoke workflows run the explicit release contract on supported triggers", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const smoke = await readWorkflow(".github/workflows/nsis-smoke.yml");
  const frontendSteps = workflowSteps(ci, "frontend").steps;
  const smokeSteps = workflowSteps(smoke, "nsis-bundle-smoke").steps;

  assert.ok(frontendSteps.some((step) => step.run === "pnpm test:release-contract"));
  assert.ok(smokeSteps.some((step) => step.run === "pnpm test:release-contract"));
  assert.equal(smoke.on.workflow_dispatch, null);
  assert.ok(smoke.on.schedule);
  assert.equal(smoke.on.release, undefined);
});

test("CI workflow token defaults to read-only repository contents", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  assert.deepEqual(
    ci.permissions,
    { contents: "read" },
    "CI must declare only top-level contents: read permissions",
  );
});

test("reviewed workflow inventory covers every workflow YAML file", async () => {
  assert.deepEqual(
    await discoveredWorkflowPaths(),
    [...workflowPaths].sort(),
    "every workflow file must be added to the reviewed action and cache policy inventory",
  );
});

test("all CI, smoke, and release actions use exact reviewed commit pins", async () => {
  for (const workflowPath of workflowPaths) {
    assertReviewedActionPins(await readWorkflow(workflowPath), workflowPath);
  }
});

test("CI fails closed if any glib version becomes Windows-reachable", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const { steps: rustSteps } = workflowSteps(ci, "rust");
  const boundaryStep = rustSteps.find(
    (step) => step.name === "Verify Windows advisory boundary",
  );
  assert.ok(boundaryStep, "CI rust job must verify the target-specific glib boundary");
  assert.equal(
    normalizedRunBody(boundaryStep.run),
    reviewedWindowsGraphBoundaryRun,
    "the Windows glib graph guard must match the reviewed fail-closed script",
  );
});

test("CI runs the checksum-verified RustSec cargo-audit 0.22.2 binary", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const { steps: rustSteps } = workflowSteps(ci, "rust");
  const allRunScripts = rustSteps.map((step) => String(step.run ?? "")).join("\n");
  const auditStep = rustSteps.find((step) => step.name === "cargo audit");

  assert.doesNotMatch(
    allRunScripts,
    /\bcargo(?:\.exe)?\s+install\b[^\r\n]*\bcargo-audit\b/i,
    "CI must not compile cargo-audit from source",
  );
  assert.ok(auditStep, "CI rust job must download the pinned cargo-audit binary");
  assert.equal(
    normalizedRunBody(auditStep.run),
    reviewedCargoAuditRun,
    "the cargo-audit download, checksum, extraction, version, and invocation must match the reviewed script",
  );
});
