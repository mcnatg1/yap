import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  assertExactCacheRestores,
  assertExactPnpmStoreBindings,
  assertSafeCachePaths,
  assertTrustedMainOnlyCacheSaves,
} from "./cache-policy.mjs";
import {
  exactCacheKeys,
  pnpmStoreBindingScriptPath,
  reviewedActions,
  reviewedPnpmStoreBindingScript,
  workflowPaths,
} from "./workflow-policy.mjs";
import {
  normalizedRunBody,
  readRepoFile,
  readWorkflow,
  repoRoot,
} from "./workflow-access.mjs";

test("workflow caches restore only exact dependency downloads from safe paths", async () => {
  for (const workflowPath of workflowPaths) {
    assertExactCacheRestores(await readWorkflow(workflowPath), workflowPath);
  }
});

test("every pnpm cache is bound to the exact store consumed by pnpm", async () => {
  for (const workflowPath of workflowPaths) {
    assertExactPnpmStoreBindings(await readWorkflow(workflowPath), workflowPath);
  }
});

test("pnpm store binding script matches the reviewed fail-closed implementation", async () => {
  assert.equal(
    normalizedRunBody(await readRepoFile(pnpmStoreBindingScriptPath)),
    reviewedPnpmStoreBindingScript,
  );
});

test("reviewed pnpm store binding publishes the store it verifies", {
  skip: process.platform !== "win32",
}, async () => {
  const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), "yap-pnpm-store-binding-"));
  const githubEnv = path.join(fixtureRoot, "github-env.txt");
  await writeFile(githubEnv, "");
  try {
    const result = spawnSync(
      "pwsh.exe",
      ["-NoProfile", "-NonInteractive", "-File", path.join(repoRoot, pnpmStoreBindingScriptPath)],
      {
        cwd: path.join(repoRoot, "desktop"),
        encoding: "utf8",
        env: { ...process.env, GITHUB_ENV: githubEnv },
        timeout: 10_000,
      },
    );
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    const prefix = "PNPM_CONFIG_STORE_DIR=";
    const published = (await readFile(githubEnv, "utf8")).trim();
    assert.ok(published.startsWith(prefix), "binding did not publish PNPM_CONFIG_STORE_DIR");
    const expectedStore = path.join(process.env.LOCALAPPDATA, "pnpm", "store", "v11");
    assert.equal(
      path.normalize(published.slice(prefix.length)).toLowerCase(),
      path.normalize(expectedStore).toLowerCase(),
    );
  } finally {
    await rm(fixtureRoot, { recursive: true, force: true });
  }
});

test("pnpm cache policy rejects consumers that run before restore", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const steps = unsafe.jobs.frontend.steps;
  const installIndex = steps.findIndex((step) => step.run === "pnpm install --frozen-lockfile");
  const [install] = steps.splice(installIndex, 1);
  const bindingIndex = steps.findIndex((step) => step.name === "Bind pnpm dependency store");
  steps.splice(bindingIndex, 0, install);
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /restore pnpm before (?:its reviewed install|every pnpm consumer)/,
  );
});

test("pnpm cache policy rejects store overrides after binding", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const install = unsafe.jobs.frontend.steps.find(
    (step) => step.run === "pnpm install --frozen-lockfile",
  );
  install.env = { PNPM_CONFIG_STORE_DIR: "C:\\unsafe-pnpm-store" };
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /must not override PNPM_CONFIG_STORE_DIR/,
  );
});

test("pnpm cache policy rejects inline npm-compatible store overrides", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const audit = unsafe.jobs.frontend.steps.find(
    (step) => step.run === "pnpm audit --audit-level high",
  );
  audit.run = "$env:NPM_CONFIG_STORE_DIR = 'C:\\unsafe-pnpm-store'\npnpm audit --audit-level high";
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /must not override PNPM_CONFIG_STORE_DIR/,
  );
});

test("pnpm cache policy rejects a conditional binding step", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const binding = unsafe.jobs.frontend.steps.find(
    (step) => step.name === "Bind pnpm dependency store",
  );
  binding.if = "${{ false }}";
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /must not be conditional/,
  );
});

test("pnpm cache policy recognizes quoted corepack consumers", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const steps = unsafe.jobs.frontend.steps;
  const bindingIndex = steps.findIndex((step) => step.name === "Bind pnpm dependency store");
  steps.splice(bindingIndex, 0, { run: "corepack \"pnpm\" audit --audit-level high" });
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /restore pnpm before every pnpm consumer/,
  );
});

test("pnpm cache policy recognizes quoted pnpx consumers", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const steps = unsafe.jobs.frontend.steps;
  const bindingIndex = steps.findIndex((step) => step.name === "Bind pnpm dependency store");
  steps.splice(bindingIndex, 0, { run: "& \"pnpx\" some-tool" });
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /restore pnpm before every pnpm consumer/,
  );
});

test("pnpm cache policy rejects Windows cache-home overrides", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  for (const variable of ["USERPROFILE", "HOMEDRIVE", "HOMEPATH", "HOME"]) {
    const unsafe = structuredClone(ci);
    const restore = unsafe.jobs.frontend.steps.find(
      (step) => step.uses === reviewedActions.cacheRestore
        && step.with?.key === exactCacheKeys.pnpm,
    );
    restore.env = { [variable]: "C:\\alternate-home" };
    assert.throws(
      () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
      /must not override Windows cache-home resolution/,
      variable,
    );
  }
});

test("pnpm cache policy rejects cache-home writes through GITHUB_ENV", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const steps = unsafe.jobs.frontend.steps;
  const restoreIndex = steps.findIndex(
    (step) => step.uses === reviewedActions.cacheRestore
      && step.with?.key === exactCacheKeys.pnpm,
  );
  steps.splice(restoreIndex, 0, {
    run: "\"USERPROFILE=C:\\alternate-home\" | Out-File -FilePath $env:GITHUB_ENV -Append",
  });
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /must not alter Windows cache-home resolution/,
  );
});

test("pnpm cache policy rejects pnpm setup version drift", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const setup = unsafe.jobs.frontend.steps.find(
    (step) => step.uses === reviewedActions.setupPnpm,
  );
  setup.with.version = "11.6.0";
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /pnpm 11\.7\.0 setup/,
  );
});

test("pnpm cache policy rejects a second setup after binding", async () => {
  const ci = await readWorkflow(".github/workflows/ci.yml");
  const unsafe = structuredClone(ci);
  const steps = unsafe.jobs.frontend.steps;
  const bindingIndex = steps.findIndex((step) => step.name === "Bind pnpm dependency store");
  steps.splice(bindingIndex + 1, 0, {
    uses: reviewedActions.setupPnpm,
    with: { version: "11.7.0" },
  });
  assert.throws(
    () => assertExactPnpmStoreBindings(unsafe, ".github/workflows/ci.yml"),
    /exactly one pnpm 11\.7\.0 setup/,
  );
});

test("only successful trusted main CI pushes may save restored dependency caches", async () => {
  for (const workflowPath of workflowPaths) {
    assertTrustedMainOnlyCacheSaves(await readWorkflow(workflowPath), workflowPath);
  }
});

test("cache policy rejects dynamic pnpm paths and truthy condition wrappers", () => {
  const dynamicPnpmPath = "${{ steps.resolve-store.outputs.path }}";
  assert.throws(
    () => assertSafeCachePaths({
      with: { key: exactCacheKeys.pnpm, path: dynamicPnpmPath },
    }, "pnpm", "fixture pnpm"),
    /exact reviewed pnpm 11 store directory/,
  );

  const unsafeWorkflow = {
    jobs: {
      frontend: {
        steps: [
          {
            id: "pnpm-cache",
            uses: reviewedActions.cacheRestore,
            with: { key: exactCacheKeys.pnpm, path: dynamicPnpmPath },
          },
          { run: "pnpm test" },
          {
            if: "success() && format('{0}', github.event_name == 'push') && format('{0}', github.ref == 'refs/heads/main') && steps.pnpm-cache.outputs.cache-hit != 'true'",
            uses: reviewedActions.cacheSave,
            with: {
              key: "${{ steps.pnpm-cache.outputs.cache-primary-key }}",
              path: dynamicPnpmPath,
            },
          },
        ],
      },
    },
  };
  assert.throws(
    () => assertTrustedMainOnlyCacheSaves(unsafeWorkflow, ".github/workflows/ci.yml"),
    /exact trusted-main success gate/,
  );

  const toleratedFailureWorkflow = structuredClone(unsafeWorkflow);
  toleratedFailureWorkflow.jobs.frontend.steps[0].with.path = "~\\AppData\\Local\\pnpm\\store\\v11";
  toleratedFailureWorkflow.jobs.frontend.steps[1]["continue-on-error"] = true;
  toleratedFailureWorkflow.jobs.frontend.steps[2].with.path = "~\\AppData\\Local\\pnpm\\store\\v11";
  toleratedFailureWorkflow.jobs.frontend.steps[2].if = "success() && github.event_name == 'push' && github.ref == 'refs/heads/main' && steps.pnpm-cache.outputs.cache-hit != 'true'";
  assert.throws(
    () => assertTrustedMainOnlyCacheSaves(toleratedFailureWorkflow, ".github/workflows/ci.yml"),
    /must not tolerate a failed check before saving caches/,
  );
});
