import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { assertRequiredSpecPolicy } from "../wdio/required-spec-policy.mjs";

const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..");

async function readRepoFile(relativePath) {
  return readFile(path.join(repoRoot, relativePath), "utf8");
}

function normalizeNewlines(text) {
  return text.replaceAll("\r\n", "\n");
}

function workflowJob(workflow, name) {
  const normalized = normalizeNewlines(workflow);
  const marker = `\n  ${name}:\n`;
  const start = normalized.indexOf(marker);
  assert.notEqual(start, -1, `workflow is missing the ${name} job`);
  const bodyStart = start + marker.length;
  const nextJob = normalized.slice(bodyStart).search(/\n  [a-zA-Z0-9_-]+:\n/);
  return nextJob === -1
    ? normalized.slice(bodyStart)
    : normalized.slice(bodyStart, bodyStart + nextJob);
}

function runNodeScript(relativePath, args = []) {
  return spawnSync(process.execPath, [path.join(repoRoot, relativePath), ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

test("required native WDIO resolves all deterministic specs and forbids focus or skips", async () => {
  const ci = await readRepoFile(".github/workflows/ci.yml");
  const job = workflowJob(ci, "native-wdio");
  const { config } = await import("../wdio.required.conf.ts");
  const { config: hardwareConfig } = await import("../wdio.hardware.conf.ts");
  const requiredSpecs = config.specs.map((spec) => path.basename(spec)).sort();

  assert.deepEqual(requiredSpecs, ["live-overlay.spec.js", "smoke.spec.js"]);
  assert.equal(config.bail, 1);
  assert.notEqual(config.logLevel, "trace");
  assert.equal(config.mochaOpts.forbidOnly, true);
  assert.equal(config.mochaOpts.forbidPending, true);
  assert.deepEqual(
    hardwareConfig.specs.map((spec) => path.basename(spec)),
    ["live-overlay.hardware.spec.js"],
  );
  assert.equal(hardwareConfig.mochaOpts.forbidOnly, true);
  assert.match(job, /runs-on: windows-latest/);
  assert.match(job, /node-version:\s*["']?24\.14\.0/);
  assert.match(job, /x86_64-pc-windows-msvc/);
  assert.match(job, /pnpm test:desktop:build/);
  assert.match(job, /wdio run \.\/tests\/wdio\.required\.conf\.ts/);
  assert.match(job, /if: failure\(\)/);
  assert.match(job, /desktop\/tests\/results/);
  assert.doesNotMatch(job, /test:desktop:all|hardware\.spec\.js/);

  const requiredSources = await Promise.all(config.specs.map((spec) => readFile(spec, "utf8")));
  for (const [index, source] of requiredSources.entries()) {
    assert.doesNotThrow(() => assertRequiredSpecPolicy(source, requiredSpecs[index]));
  }
  for (const forbidden of ["it.skip('x', fn)", "describe.only('x', fn)", "this.skip()", "fit('x', fn)"]) {
    assert.throws(() => assertRequiredSpecPolicy(forbidden, "fixture"), /focused or skipped/);
  }
  const combined = requiredSources.join("\n");
  assert.match(combined, /opens as a compact system overlay and refuses direct close/);
  assert.match(combined, /allows live status, rejects privileged commands, and survives close attempts/);
  assert.match(combined, /keeps main alive when closed and restores it from the overlay/);
  assert.match(combined, /preserves the probe error after a hidden-state failure/);
  assert.match(combined, /reports an enforced CSP violation/);

  const optionalCapture = await readRepoFile("desktop/tests/wdio/live-overlay.hardware.spec.js");
  assert.match(optionalCapture, /captures and saves one session entirely from the overlay context/);
  assert.match(optionalCapture, /this\.skip\(\)/);
});

test("installer smoke stays manual or scheduled and pre-publication provenance is separate", async () => {
  const smokeWorkflow = normalizeNewlines(await readRepoFile(".github/workflows/nsis-smoke.yml"));
  const prepublishWorkflow = normalizeNewlines(
    await readRepoFile(".github/workflows/prepublish-provenance.yml"),
  );
  const smokeScript = await readRepoFile("desktop/tests/scripts/smoke-nsis.ps1");
  const smokeHelpers = await readRepoFile("desktop/tests/scripts/nsis-smoke-helpers.psm1");

  assert.match(smokeWorkflow, /workflow_dispatch:/);
  assert.match(smokeWorkflow, /schedule:/);
  assert.doesNotMatch(smokeWorkflow, /\n\s*release:/);
  assert.match(smokeWorkflow, /node-version:\s*["']?24\.14\.0/);
  assert.match(smokeWorkflow, /x86_64-pc-windows-msvc/);
  assert.match(smokeWorkflow, /pnpm tauri build --bundles nsis/);
  assert.match(smokeWorkflow, /smoke-nsis\.ps1/);
  assert.match(smokeWorkflow, /desktop\/tests\/results\/nsis-smoke/);

  assert.match(prepublishWorkflow, /workflow_dispatch:/);
  assert.match(prepublishWorkflow, /workflow_call:/);
  assert.doesNotMatch(prepublishWorkflow, /\n\s*release:/);
  assert.match(prepublishWorkflow, /node-version:\s*["']?24\.14\.0/);
  assert.match(prepublishWorkflow, /assert-third-party-provenance\.mjs --require-verified/);

  assert.match(smokeScript, /Invoke-ProcessWithDeadline/);
  assert.match(smokeScript, /Assert-NoReparsePoints/);
  assert.match(smokeScript, /Assert-NoProcessesUnderPath/);
  assert.match(smokeScript, /Remove-ValidatedTree/);
  assert.match(smokeScript, /try\s*{\s*Write-Evidence/);
  assert.match(smokeScript, /finally\s*{[\s\S]*Write-Evidence/);
  assert.doesNotMatch(`${smokeScript}\n${smokeHelpers}`, /SilentlyContinue|Start-Process[^\n]*-Wait/);
});

test("provenance manifest records exact files while truthfully remaining unverified", async () => {
  const manifest = JSON.parse(await readRepoFile("THIRD_PARTY_PROVENANCE.json"));
  const tauriConfig = JSON.parse(await readRepoFile("desktop/src-tauri/tauri.conf.json"));
  const notice = await readRepoFile("THIRD_PARTY_NOTICES.md");
  const freeFlow = manifest.sources.find(({ id }) => id === "freeflow");

  assert.equal(manifest.schemaVersion, 1);
  assert.equal(freeFlow.repository, "https://github.com/zachlatta/freeflow");
  assert.equal(freeFlow.license, "MIT");
  assert.deepEqual(freeFlow.revision, { status: "unverified", value: null });
  assert.equal(freeFlow.notice, "THIRD_PARTY_NOTICES.md");
  assert.equal(freeFlow.files.length, 2);
  assert.deepEqual(
    freeFlow.files.map(({ path: filePath }) => filePath).sort(),
    [
      "desktop/src-tauri/src/audio/preprocess.rs",
      "desktop/src/components/live/live-overlay.tsx",
    ].sort(),
  );
  for (const file of freeFlow.files) {
    assert.match(file.sha256, /^[0-9a-f]{64}$/);
    assert.equal(typeof file.path, "string");
  }
  assert.match(notice, /^## FreeFlow$/m);
  assert.match(notice, /https:\/\/github\.com\/zachlatta\/freeflow/);
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
    ["--require-verified"],
  );
  assert.notEqual(prepublish.status, 0);
  assert.match(prepublish.stderr, /revision is explicitly unverified/i);
});

test("NSIS path and process helpers pass their focused PowerShell unit suite", () => {
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
