import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { access, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
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
const reviewedActions = Object.freeze({
  cacheRestore: "actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
  cacheSave: "actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9",
  checkout: "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0",
  downloadArtifact: "actions/download-artifact@37930b1c2abaa49bbe596cd826c3c89aef350131",
  setupNode: "actions/setup-node@48b55a011bda9f5d6aeb4c2d9c7362e8dae4041e",
  setupPnpm: "pnpm/action-setup@0ebf47130e4866e96fce0953f49152a61190b271",
  setupPython: "actions/setup-python@ece7cb06caefa5fff74198d8649806c4678c61a1",
  setupRust: "dtolnay/rust-toolchain@4be7066ada62dd38de10e7b70166bc74ed198c30",
  uploadArtifact: "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
});
const reviewedActionUses = new Set(Object.values(reviewedActions));
const workflowPaths = Object.freeze([
  ".github/workflows/ci.yml",
  ".github/workflows/nsis-smoke.yml",
  ".github/workflows/release.yml",
]);
const exactCacheKeys = Object.freeze({
  cargo: "cargo-deps-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('desktop/src-tauri/Cargo.lock') }}",
  playwright: "playwright-v1-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('desktop/pnpm-lock.yaml') }}",
  pnpm: "pnpm-store-v11-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('desktop/pnpm-lock.yaml') }}",
});
const expectedCacheFamilies = Object.freeze({
  ".github/workflows/ci.yml": Object.freeze({
    frontend: Object.freeze(["playwright", "pnpm"]),
    "native-wdio": Object.freeze(["cargo", "pnpm"]),
    rust: Object.freeze(["cargo"]),
  }),
  ".github/workflows/nsis-smoke.yml": Object.freeze({
    "nsis-bundle-smoke": Object.freeze(["cargo", "pnpm"]),
  }),
  ".github/workflows/release.yml": Object.freeze({
    "build-nsis": Object.freeze(["cargo", "pnpm"]),
  }),
});
const pnpmStoreBindingScriptPath = "desktop/tests/scripts/bind-pnpm-cache-store.ps1";
const reviewedPnpmStoreBindingInvocation = String.raw`
& "$env:GITHUB_WORKSPACE\desktop\tests\scripts\bind-pnpm-cache-store.ps1"
`.trim();
const reviewedPnpmStoreBindingScript = String.raw`
#requires -Version 7.4
#requires -PSEdition Core

$ErrorActionPreference = "Stop"
$localAppData = [Environment]::GetFolderPath(
  [Environment+SpecialFolder]::LocalApplicationData
)
$expectedStore = [IO.Path]::GetFullPath(
  (Join-Path $localAppData "pnpm\store\v11")
)
$cacheStore = [IO.Path]::GetFullPath(
  (Join-Path $HOME "AppData\Local\pnpm\store\v11")
)
if ($expectedStore -ine $cacheStore) {
  throw "The reviewed pnpm cache path does not match Windows LocalApplicationData."
}
$env:PNPM_CONFIG_STORE_DIR = $expectedStore
$actualStoreOutput = @(pnpm store path)
if ($LASTEXITCODE -ne 0 -or $actualStoreOutput.Count -ne 1) {
  throw "Failed to resolve the configured pnpm dependency store."
}
$actualStore = [IO.Path]::GetFullPath(([string]$actualStoreOutput[0]).Trim())
if ($actualStore -ine $expectedStore) {
  throw "pnpm did not accept the reviewed dependency store."
}
"PNPM_CONFIG_STORE_DIR=$expectedStore" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
`.trim();
const releaseActionUses = new Set([
  reviewedActions.cacheRestore,
  reviewedActions.checkout,
  reviewedActions.downloadArtifact,
  reviewedActions.setupNode,
  reviewedActions.setupPnpm,
  reviewedActions.setupRust,
  reviewedActions.uploadArtifact,
]);
const reviewedWindowsGraphBoundaryRun = String.raw`
$ErrorActionPreference = "Stop"
$windowsPackages = @(cargo tree --locked --offline --target x86_64-pc-windows-msvc --prefix none --format "{p}")
if ($LASTEXITCODE -ne 0) {
  throw "Unable to inspect the locked Windows dependency graph."
}
$windowsGlibPackages = @($windowsPackages | Where-Object { $_ -match '^glib v' })
if ($windowsGlibPackages.Count -ne 0) {
  throw "glib became reachable on Windows; reevaluate GHSA-wrw7-89jp-8q8g: $($windowsGlibPackages -join ', ')"
}
`.trim();
const reviewedCargoAuditRun = String.raw`
$ErrorActionPreference = "Stop"
$archive = Join-Path $env:RUNNER_TEMP "cargo-audit-x86_64-pc-windows-msvc-v0.22.2.zip"
$url = "https://github.com/RustSec/rustsec/releases/download/cargo-audit/v0.22.2/cargo-audit-x86_64-pc-windows-msvc-v0.22.2.zip"
$extractRoot = Join-Path $env:RUNNER_TEMP "cargo-audit-0.22.2"
Invoke-WebRequest -Uri $url -OutFile $archive
$actualSha256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -cne "0a7316540862c13d954f648917ceacca593747baed6eec180fafa590be2710ab") {
  throw "Pinned cargo-audit archive hash mismatch."
}
Expand-Archive -LiteralPath $archive -DestinationPath $extractRoot -Force
$cargoAudit = Join-Path $extractRoot "cargo-audit-x86_64-pc-windows-msvc-v0.22.2\cargo-audit.exe"
if (-not (Test-Path -LiteralPath $cargoAudit -PathType Leaf)) {
  throw "Pinned cargo-audit executable was not extracted."
}
$cargoAuditVersion = & $cargoAudit --version
if ($LASTEXITCODE -ne 0 -or $cargoAuditVersion -cne "cargo-audit 0.22.2") {
  throw "Pinned cargo-audit executable has an unexpected version."
}
# Policy: cargo-audit warnings from Tauri's target-all desktop
# transitive crates are allowed for now. Vulnerabilities fail CI.
& $cargoAudit audit --target-os windows --target-arch x86_64
if ($LASTEXITCODE -ne 0) { throw "cargo-audit failed." }
`.trim();

async function readRepoFile(relativePath) {
  return readFile(path.join(repoRoot, relativePath), "utf8");
}

async function readWorkflow(relativePath) {
  return parseYaml(await readRepoFile(relativePath));
}

async function discoveredWorkflowPaths() {
  const workflowsRoot = path.join(repoRoot, ".github", "workflows");
  const entries = await readdir(workflowsRoot, { withFileTypes: true });
  return entries
    .filter((entry) => entry.isFile() && /\.ya?ml$/i.test(entry.name))
    .map((entry) => `.github/workflows/${entry.name}`)
    .sort();
}

function normalizedRunBody(source) {
  return String(source).replaceAll("\r\n", "\n").trim();
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

function workflowStepEntries(workflow) {
  return Object.entries(workflow.jobs ?? {}).flatMap(([jobName, job]) =>
    (job.steps ?? []).map((step, index) => ({ index, jobName, step }))
  );
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

function assertSafeCachePaths(step, family, label) {
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
    assert.equal(paths.length, 1, `${label} must cache only Playwright browser downloads`);
    assert.equal(
      paths[0],
      "~\\AppData\\Local\\ms-playwright",
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

function assertReviewedUse(usesValue, label) {
  const uses = String(usesValue);
  assert.match(
    uses,
    /@[0-9a-f]{40}$/,
    `${label} must use an exact 40-character commit SHA: ${uses}`,
  );
  assert.ok(
    reviewedActionUses.has(uses),
    `${label} is not pinned to a reviewed revision: ${uses}`,
  );
}

function assertReviewedActionPins(workflow, workflowPath) {
  for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
    if (job.uses) {
      assertReviewedUse(job.uses, `${workflowPath} ${jobName} reusable workflow`);
    }
    for (const step of job.steps ?? []) {
      if (step.uses) assertReviewedUse(step.uses, `${workflowPath} ${jobName} action`);
    }
  }
}

function assertExactCacheRestores(workflow, workflowPath) {
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

function assertExactPnpmStoreBindings(workflow, workflowPath) {
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

function assertTrustedMainOnlyCacheSaves(workflow, workflowPath) {
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
      assert.equal(
        saves.length,
        0,
        `${workflowPath} ${jobName} must be restore-only`,
      );
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
      const isAnotherCacheSave = followingStep.uses === reviewedActions.cacheSave;
      assert.ok(
        isAnotherCacheSave,
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
      const outerExpression = condition.match(/^\$\{\{\s*([\s\S]*?)\s*\}\}$/);
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
      with: {
        key: exactCacheKeys.pnpm,
        path: dynamicPnpmPath,
      },
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
            with: {
              key: exactCacheKeys.pnpm,
              path: dynamicPnpmPath,
            },
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
  const auditRun = String(auditStep.run);
  assert.equal(
    normalizedRunBody(auditRun),
    reviewedCargoAuditRun,
    "the cargo-audit download, checksum, extraction, version, and invocation must match the reviewed script",
  );
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

  const resolveCheckout = resolveSteps.find((step) => step.uses === reviewedActions.checkout);
  assert.equal(resolveCheckout.with.ref, "${{ github.event.repository.default_branch }}");
  assert.equal(resolveCheckout.with["fetch-depth"], 0);
  assert.equal(resolveCheckout.with["persist-credentials"], false);
  const resolve = resolveSteps.find((step) => step.name === "Resolve immutable commit from default branch");
  assert.match(resolve.run, /\^\[0-9a-fA-F\]\{40\}\$/);
  assert.match(resolve.run, /merge-base --is-ancestor/);
  assert.doesNotMatch(resolve.run, /git fetch|refs\/remotes\/origin/);

  const buildCheckout = buildSteps.find((step) => step.uses === reviewedActions.checkout);
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
  assert.match(buildSteps[captureEnvironment].run, /YAP_RELEASE_POWERSHELL_EDITION/);
  assert.match(buildSteps[captureEnvironment].run, /YAP_RELEASE_POWERSHELL_VERSION/);
  assert.match(buildSteps[captureEnvironment].run, /\$PSVersionTable\.PSEdition/);
  assert.match(buildSteps[captureEnvironment].run, /\$PSVersionTable\.PSVersion/);
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
  assert.deepEqual(new Set(releaseUses), releaseActionUses);
  for (const action of releaseUses) assert.match(action, /@[0-9a-f]{40}$/);
  const verify = publishSteps.find((step) => step.name === "Verify downloaded payload binding");
  assert.match(verify.run, /Get-FileHash/);
  assert.match(verify.run, /VERIFIED_SHA/);
  assert.match(verify.run, /RELEASE_TAG/);
  assert.match(verify.run, /metadata\.version/);
  assert.match(verify.run, /buildEnvironment\.runner\.imageOs/);
  assert.match(verify.run, /"powershellEdition"/);
  assert.match(verify.run, /"powershellVersion"/);
  assert.match(verify.run, /powershellEdition -cne "Core"/);
  assert.match(verify.run, /\$powerShellVersion -lt \[version\]"7\.4"/);
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

test("Windows release automation requires PowerShell 7.4 Core", async () => {
  const powerShellFiles = [
    "desktop/tests/scripts/bind-pnpm-cache-store.ps1",
    "desktop/tests/scripts/build-nsis-test.ps1",
    "desktop/tests/scripts/native-window-recovery.test.ps1",
    "desktop/tests/scripts/nsis-smoke-helpers.psm1",
    "desktop/tests/scripts/nsis-smoke-helpers.test.ps1",
    "desktop/tests/scripts/smoke-nsis-local.ps1",
    "desktop/tests/scripts/smoke-nsis-production-delete.ps1",
    "desktop/tests/scripts/smoke-nsis-test-delete.ps1",
    "desktop/tests/scripts/smoke-nsis.ps1",
    "desktop/tests/scripts/windows-contained-process-fixture.ps1",
    "desktop/tests/scripts/windows-contained-process.contract.test.ps1",
    "desktop/tests/scripts/windows-contained-process.integration.test.ps1",
    "desktop/tests/wdio/native-window-recovery.psm1",
  ];
  const trackedPowerShellFiles = execFileSync(
    "git",
    ["ls-files", "--", "*.ps1", "*.psm1"],
    { cwd: repoRoot, encoding: "utf8" },
  )
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)
    .sort();
  assert.deepEqual(
    trackedPowerShellFiles,
    [...powerShellFiles].sort(),
    "PowerShell runtime contract inventory must cover every tracked script and module",
  );
  const runtimeRequirement =
    /^#requires -Version 7\.4\r?\n#requires -PSEdition Core\b/i;

  for (const relativePath of powerShellFiles) {
    const source = await readRepoFile(relativePath);
    assert.match(
      source,
      runtimeRequirement,
      `${relativePath} must fail fast outside PowerShell Core 7.4 or newer`,
    );
  }

  const nsisBuildScript = await readRepoFile(
    "desktop/tests/scripts/build-nsis-test.ps1",
  );
  assert.match(nsisBuildScript, /Get-Command "pnpm"/);
  assert.match(nsisBuildScript, /\("\.cmd", "\.exe"\)/);
  assert.match(nsisBuildScript, /Select-Object -First 1/);
  assert.match(nsisBuildScript, /-Environment @\{ CARGO_TARGET_DIR = \$targetRoot \}/);
  assert.match(nsisBuildScript, /\$process\.WaitForExit\(\$buildTimeoutMilliseconds\)/);
  assert.match(nsisBuildScript, /\$process\.Kill\(\$true\)/);
  assert.match(nsisBuildScript, /\$process\.WaitForExit\(10000\)/);
  assert.doesNotMatch(nsisBuildScript, /\s-Wait\b/);
  assert.doesNotMatch(nsisBuildScript, /SetEnvironmentVariable/);
  assert.doesNotMatch(nsisBuildScript, /\$env:CARGO_TARGET_DIR\s*=/i);

  const nsisHelpers = await readRepoFile(
    "desktop/tests/scripts/nsis-smoke-helpers.psm1",
  );
  assert.match(nsisHelpers, /\[Yap\.NsisSmoke\.LaunchRequest\]::Create\(/);
  assert.match(nsisHelpers, /ConvertTo-ChildEnvironment/);
  assert.match(nsisHelpers, /\$childEnvironment\r?\n\s*\)/);
  assert.doesNotMatch(nsisHelpers, /SetEnvironmentVariable/);

  const legacyWindowsPowerShell = ["power", "shell.exe"].join("");
  const runtimeSelectors = [
    "desktop/package.json",
    "desktop/tests/scripts/nsis-smoke-helpers.test.ps1",
    "desktop/tests/scripts/release-evidence.contract.mjs",
    "desktop/tests/wdio/live-overlay.spec.js",
  ];
  for (const relativePath of runtimeSelectors) {
    const source = (await readRepoFile(relativePath)).toLowerCase();
    assert.equal(
      source.includes(legacyWindowsPowerShell),
      false,
      `${relativePath} still selects legacy Windows PowerShell`,
    );
  }
  const containedIntegration = await readRepoFile(
    "desktop/tests/scripts/windows-contained-process.integration.test.ps1",
  );
  assert.match(containedIntegration, /Join-Path \$PSHOME "pwsh\.exe"/);
  const releaseContract = await readRepoFile(
    "desktop/tests/scripts/release-evidence.contract.mjs",
  );
  assert.match(releaseContract, /spawnSync\(\s*"pwsh\.exe"/);
  const liveOverlaySpec = await readRepoFile(
    "desktop/tests/wdio/live-overlay.spec.js",
  );
  assert.match(liveOverlaySpec, /execFileAsync\(\s*"pwsh\.exe"/);

  const packageJson = JSON.parse(await readRepoFile("desktop/package.json"));
  for (const scriptName of [
    "build:nsis:test",
    "test:nsis:local",
    "test:nsis:test-delete",
  ]) {
    assert.match(
      packageJson.scripts[scriptName],
      /(?:^|\s)pwsh\.exe\s/i,
      `${scriptName} must select PowerShell 7 explicitly`,
    );
  }

  for (const relativePath of [
    ".github/workflows/ci.yml",
    ".github/workflows/nsis-smoke.yml",
    ".github/workflows/release.yml",
  ]) {
    const workflow = await readWorkflow(relativePath);
    for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
      const runsOn = String(job["runs-on"] ?? "");
      if (!runsOn.startsWith("windows-")) continue;

      assert.equal(
        job.defaults?.run?.shell,
        "pwsh",
        `${relativePath} ${jobName} must explicitly default run steps to PowerShell Core`,
      );
      const guard = job.steps?.find(
        (step) => step.name === "Verify PowerShell 7.4 Core",
      );
      assert.ok(
        guard,
        `${relativePath} ${jobName} must validate its isolated runner's PowerShell runtime`,
      );
      assert.equal(guard.shell, "pwsh");
      assert.equal(
        job.steps.find((step) => step.run),
        guard,
        `${relativePath} ${jobName} must validate PowerShell before any other run step`,
      );
      assert.match(
        guard.run,
        /\$PSVersionTable\.PSEdition\s+-cne\s+["']Core["']/,
      );
      assert.match(
        guard.run,
        /\$PSVersionTable\.PSVersion\s+-lt\s+\[version\]["']7\.4["']/,
      );
      assert.match(guard.run, /\bthrow\b/);
      assert.equal(
        job.steps.some((step) =>
          /^powershell(?:\.exe)?$/i.test(String(step.shell ?? ""))),
        false,
        `${relativePath} ${jobName} overrides a run step back to legacy PowerShell`,
      );
    }
  }

  const ciWorkflow = await readWorkflow(".github/workflows/ci.yml");
  const compatibilityJob = ciWorkflow.jobs?.frontend;
  assert.ok(compatibilityJob, "required frontend CI job is missing");
  assert.equal(compatibilityJob.env.POWERSHELL_74_VERSION, "7.4.17");
  assert.equal(compatibilityJob.env.POWERSHELL_TELEMETRY_OPTOUT, "1");
  assert.equal(
    compatibilityJob.env.POWERSHELL_74_SHA256,
    "266479A93B82CD0DC0F043419388FD4A738A51082821C301FFF497212FAF6760",
  );
  const installPowerShell = compatibilityJob.steps.find(
    (step) => step.name === "Install pinned PowerShell 7.4 runtime",
  );
  assert.match(installPowerShell.run, /PowerShell\/PowerShell\/releases\/download/);
  assert.match(installPowerShell.run, /Get-FileHash/);
  assert.match(installPowerShell.run, /POWERSHELL_74_SHA256/);
  assert.match(installPowerShell.run, /Expand-Archive/);
  assert.match(installPowerShell.run, /GITHUB_PATH/);
  const runCompatibilitySuite = compatibilityJob.steps.find(
    (step) => step.name === "Run focused suite under PowerShell 7.4",
  );
  assert.match(runCompatibilitySuite.run, /YAP_POWERSHELL_74/);
  assert.match(runCompatibilitySuite.run, /nsis-smoke-helpers\.test\.ps1/);
  assert.match(runCompatibilitySuite.run, /Language\.Parser/);
  assert.match(runCompatibilitySuite.run, /nestedRuntime/);
  assert.match(runCompatibilitySuite.run, /PSEdition -cne "Core"/);
  assert.match(runCompatibilitySuite.run, /PSVersion\.ToString\(\)/);
  assert.match(runCompatibilitySuite.run, /POWERSHELL_74_VERSION/);

  const helperModulePath = path
    .join(repoRoot, "desktop/tests/scripts/nsis-smoke-helpers.psm1")
    .replaceAll("'", "''");
  const legacyResult = spawnSync(
    legacyWindowsPowerShell,
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      `Import-Module '${helperModulePath}' -Force`,
    ],
    { cwd: repoRoot, encoding: "utf8", timeout: 10_000 },
  );
  assert.notEqual(
    legacyResult.status,
    0,
    "legacy Windows PowerShell unexpectedly loaded release automation",
  );
  assert.match(
    `${legacyResult.stdout}\n${legacyResult.stderr}`,
    /#requires[\s\S]*PowerShell 7\.4|PSEdition Core/i,
  );
});

test("NSIS helper behavior passes its focused PowerShell 7 suite", () => {
  const result = spawnSync(
    "pwsh.exe",
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
