import assert from "node:assert/strict";

import {
  exactCacheKeys,
  expectedCacheFamilies,
  reviewedActions,
  reviewedPnpmStoreBindingInvocation,
} from "./workflow-policy.mjs";
import {
  normalizedRunBody,
  workflowStepEntries,
  workflowSteps,
} from "./workflow-access.mjs";

export function assertExactCacheRestores(workflow, workflowPath) {
  const expectedByJob = expectedCacheFamilies[workflowPath];
  const actualByJob = new Map();
  for (const { jobName, step } of workflowStepEntries(workflow)) {
    const uses = String(step.uses ?? "");
    if (
      uses.startsWith("actions/setup-node@")
      || uses.startsWith("actions/setup-python@")
      || uses.startsWith("pnpm/action-setup@")
    ) {
      assert.equal(
        step.with?.cache,
        undefined,
        `${workflowPath} ${jobName} must not use an action's native save-capable cache`,
      );
      assert.equal(
        step.with?.["cache-dependency-path"],
        undefined,
        `${workflowPath} ${jobName} must not configure a native action cache dependency path`,
      );
    }
    if (uses.startsWith("actions/setup-node@")) {
      assert.equal(
        step.with?.["package-manager-cache"],
        false,
        `${workflowPath} ${jobName} must disable setup-node's implicit package-manager cache`,
      );
    }
    if (!uses.startsWith("actions/cache")) continue;
    assert.notEqual(
      uses.split("@")[0],
      "actions/cache",
      `${workflowPath} ${jobName} must not use monolithic actions/cache`,
    );
    if (uses !== reviewedActions.cacheRestore) continue;
    const label = `${workflowPath} ${jobName}`;
    const family = cacheFamily(step, label);
    assertSafeCachePaths(step, family, label);
    const families = actualByJob.get(jobName) ?? [];
    families.push(family);
    actualByJob.set(jobName, families);
  }

  assert.deepEqual(
    [...actualByJob.keys()].sort(),
    Object.keys(expectedByJob).sort(),
    `${workflowPath} restore-only cache jobs do not match the dependency policy`,
  );
  for (const [jobName, expected] of Object.entries(expectedByJob)) {
    assert.deepEqual(
      [...(actualByJob.get(jobName) ?? [])].sort(),
      [...expected].sort(),
      `${workflowPath} ${jobName} dependency cache families do not match policy`,
    );
  }
}

export function assertExactPnpmStoreBindings(workflow, workflowPath) {
  assertNoPnpmStoreEnvOverride(workflow.env, `${workflowPath} workflow`);
  for (const [jobName, families] of Object.entries(expectedCacheFamilies[workflowPath])) {
    if (!families.includes("pnpm")) continue;
    const { job, steps } = workflowSteps(workflow, jobName);
    assertNoPnpmStoreEnvOverride(job.env, `${workflowPath} ${jobName}`);
    const restoreIndex = steps.findIndex(
      (step) => step.uses === reviewedActions.cacheRestore
        && step.with?.key === exactCacheKeys.pnpm,
    );
    assert.notEqual(
      restoreIndex,
      -1,
      `${workflowPath} ${jobName} is missing the exact pnpm cache restore`,
    );
    const bindings = steps
      .map((step, index) => ({ index, step }))
      .filter(({ step }) => step.name === "Bind pnpm dependency store");
    assert.equal(
      bindings.length,
      1,
      `${workflowPath} ${jobName} must bind exactly one pnpm dependency store`,
    );
    const setups = steps
      .map((step, index) => ({ index, step }))
      .filter(({ step }) => step.uses === reviewedActions.setupPnpm);
    assert.equal(
      setups.length,
      1,
      `${workflowPath} ${jobName} must have exactly one pnpm 11.7.0 setup`,
    );
    assert.deepEqual(
      setups[0].step.with,
      { version: "11.7.0" },
      `${workflowPath} ${jobName} pnpm 11.7.0 setup must match the reviewed inputs`,
    );
    assert.ok(
      setups[0].index < bindings[0].index,
      `${workflowPath} ${jobName} must set up pnpm 11.7.0 before binding its store`,
    );
    assert.ok(
      bindings[0].index < restoreIndex,
      `${workflowPath} ${jobName} must bind pnpm before restoring its cache`,
    );
    const installs = steps
      .map((step, index) => ({ index, step }))
      .filter(({ step }) => step.run === "pnpm install --frozen-lockfile");
    assert.equal(
      installs.length,
      1,
      `${workflowPath} ${jobName} must have exactly one reviewed pnpm install`,
    );
    assert.ok(
      restoreIndex < installs[0].index,
      `${workflowPath} ${jobName} must restore pnpm before its reviewed install`,
    );
    for (const [index, step] of steps.entries()) {
      assertNoPnpmStoreEnvOverride(
        step.env,
        `${workflowPath} ${jobName} step ${step.name ?? index}`,
      );
      if (index === bindings[0].index) continue;
      const run = String(step.run ?? "");
      assert.doesNotMatch(
        run,
        /(?:PNPM|NPM)_CONFIG_STORE_DIR|\bstore-dir\b|\bstoreDir\b/i,
        `${workflowPath} ${jobName} step ${step.name ?? index} must not override PNPM_CONFIG_STORE_DIR`,
      );
      assert.doesNotMatch(
        run,
        /\b(?:USERPROFILE|HOMEDRIVE|HOMEPATH|HOME)\b/i,
        `${workflowPath} ${jobName} step ${step.name ?? index} must not alter Windows cache-home resolution`,
      );
      if (/\b(?:pnpm|pnpx)(?:\.cmd|\.exe)?\b/i.test(run)) {
        assert.ok(
          restoreIndex < index,
          `${workflowPath} ${jobName} must restore pnpm before every pnpm consumer`,
        );
      }
    }
    for (const protectedStep of [
      setups[0].step,
      bindings[0].step,
      steps[restoreIndex],
      installs[0].step,
    ]) {
      assert.ok(
        protectedStep["continue-on-error"] === undefined
          || protectedStep["continue-on-error"] === false,
        `${workflowPath} ${jobName} pnpm binding, restore, and install must fail closed`,
      );
      assert.equal(
        protectedStep.if,
        undefined,
        `${workflowPath} ${jobName} pnpm binding, restore, and install must not be conditional`,
      );
    }
    assert.equal(
      bindings[0].step.shell,
      "pwsh",
      `${workflowPath} ${jobName} must bind pnpm with PowerShell Core`,
    );
    assert.equal(
      normalizedRunBody(bindings[0].step.run),
      reviewedPnpmStoreBindingInvocation,
      `${workflowPath} ${jobName} pnpm store binding must match the reviewed fail-closed script`,
    );
  }
}

export function assertTrustedMainOnlyCacheSaves(workflow, workflowPath) {
  for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
    const steps = job.steps ?? [];
    const restores = steps
      .map((step, index) => ({ index, step }))
      .filter(({ step }) => step.uses === reviewedActions.cacheRestore);
    const saves = steps
      .map((step, index) => ({ index, step }))
      .filter(({ step }) => step.uses === reviewedActions.cacheSave);
    const maySave = workflowPath === ".github/workflows/ci.yml"
      && (jobName === "frontend" || jobName === "rust");

    if (!maySave) {
      assert.equal(saves.length, 0, `${workflowPath} ${jobName} must be restore-only`);
      continue;
    }

    assert.ok(restores.length > 0, `${workflowPath} ${jobName} has no dependency restores`);
    assert.equal(
      saves.length,
      restores.length,
      `${workflowPath} ${jobName} must save each restored dependency cache exactly once`,
    );
    const firstSaveIndex = Math.min(...saves.map(({ index }) => index));
    for (const precedingStep of steps.slice(0, firstSaveIndex)) {
      assert.ok(
        precedingStep["continue-on-error"] === undefined
          || precedingStep["continue-on-error"] === false,
        `${workflowPath} ${jobName} must not tolerate a failed check before saving caches`,
      );
    }
    for (const followingStep of steps.slice(firstSaveIndex + 1)) {
      assert.equal(
        followingStep.uses,
        reviewedActions.cacheSave,
        `${workflowPath} ${jobName} cache saves must follow every substantive check`,
      );
    }
    const pairedRestoreIds = new Set();
    for (const { index: saveIndex, step: save } of saves) {
      const keyReference = String(save.with?.key ?? "").match(
        /^\${{\s*steps\.([a-z0-9_-]+)\.outputs\.cache-primary-key\s*}}$/i,
      );
      assert.ok(
        keyReference,
        `${workflowPath} ${jobName} save must reuse a restore cache-primary-key`,
      );
      const restoreId = keyReference[1];
      const restore = restores.find(({ step }) => step.id === restoreId);
      assert.ok(restore, `${workflowPath} ${jobName} save references unknown restore ${restoreId}`);
      assert.ok(
        restore.index < saveIndex,
        `${workflowPath} ${jobName} save must run after restore ${restoreId}`,
      );
      assert.equal(
        pairedRestoreIds.has(restoreId),
        false,
        `${workflowPath} ${jobName} restore ${restoreId} is saved more than once`,
      );
      pairedRestoreIds.add(restoreId);
      assert.deepEqual(
        cachePaths(save),
        cachePaths(restore.step),
        `${workflowPath} ${jobName} save paths must match restore ${restoreId}`,
      );

      const condition = String(save.if ?? "").trim();
      const outerExpression = condition.match(/^\${{\s*([\s\S]*?)\s*}}$/);
      const normalizedCondition = (outerExpression?.[1] ?? condition)
        .replace(/\s+/g, " ")
        .trim();
      assert.equal(
        normalizedCondition,
        `success() && github.event_name == 'push' && github.ref == 'refs/heads/main' && steps.${restoreId}.outputs.cache-hit != 'true'`,
        `${workflowPath} ${jobName} save must use the exact trusted-main success gate`,
      );
    }
    assert.equal(
      pairedRestoreIds.size,
      restores.length,
      `${workflowPath} ${jobName} has an unpaired dependency restore`,
    );
  }
}

function cachePaths(step) {
  return String(step.with?.path ?? "")
    .split(/\r?\n/)
    .map((value) => value.trim())
    .filter(Boolean);
}

function normalizedCachePath(cachePath) {
  return cachePath.replaceAll("\\", "/").replace(/\/+$/, "").toLowerCase();
}

function cacheFamily(step, label) {
  const key = String(step.with?.key ?? "");
  const family = Object.entries(exactCacheKeys).find(([, expected]) => key === expected)?.[0];
  assert.ok(family, `${label} cache key must be one of the exact dependency keys; received ${key}`);
  return family;
}

export function assertSafeCachePaths(step, family, label) {
  assert.equal(
    step.with?.["restore-keys"],
    undefined,
    `${label} must not use a broad restore-keys prefix`,
  );
  const paths = cachePaths(step);
  assert.ok(paths.length > 0, `${label} has no cache paths`);
  for (const cachePath of paths) {
    assert.doesNotMatch(
      normalizedCachePath(cachePath),
      /(^|\/)(?:target|node_modules|bundle|dist|coverage|test-results?|results|playwright-report|release-evidence|artifact-seal|installers?|models?|recordings?|transcripts?|advisory-db|\.rustsec|credentials?|secrets?|tokens?|certificates?|\.env(?:\.[^/]*)?)(?:\/|$)|\.(?:sqlite3?|db|pem|pfx|p12|key)(?:\/|$)/i,
      `${label} includes mutable, sensitive, result, or release-evidence state: ${cachePath}`,
    );
  }

  if (family === "pnpm") {
    assert.deepEqual(
      paths,
      ["~\\AppData\\Local\\pnpm\\store\\v11"],
      `${label} must use the exact reviewed pnpm 11 store directory`,
    );
  } else if (family === "playwright") {
    assert.deepEqual(
      paths,
      ["~\\AppData\\Local\\ms-playwright"],
      `${label} must use the exact reviewed Playwright browser directory`,
    );
  } else {
    assert.deepEqual(
      [...paths].sort(),
      ["~/.cargo/git/db", "~/.cargo/registry/cache", "~/.cargo/registry/index"],
      `${label} must use exactly the three reviewed Cargo home dependency paths`,
    );
  }
}

function assertNoPnpmStoreEnvOverride(env, label) {
  for (const key of Object.keys(env ?? {})) {
    assert.doesNotMatch(
      key,
      /^(?:PNPM_CONFIG_STORE_DIR|NPM_CONFIG_STORE_DIR)$/i,
      `${label} must not override PNPM_CONFIG_STORE_DIR`,
    );
    assert.doesNotMatch(
      key,
      /^(?:USERPROFILE|HOMEDRIVE|HOMEPATH|HOME)$/i,
      `${label} must not override Windows cache-home resolution`,
    );
  }
}
