import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

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

test("required native WDIO smoke is strict, cached, and hardware-independent", async () => {
  const ci = await readRepoFile(".github/workflows/ci.yml");
  const job = workflowJob(ci, "native-wdio");
  const requiredConfig = await readRepoFile("desktop/tests/wdio.required.conf.ts");

  assert.match(job, /runs-on: windows-latest/);
  assert.match(job, /actions\/cache@v\d+/);
  assert.match(job, /pnpm test:desktop:build/);
  assert.match(job, /wdio run \.\/tests\/wdio\.required\.conf\.ts/);
  assert.match(job, /if: failure\(\)/);
  assert.match(job, /desktop\/tests\/results/);
  assert.doesNotMatch(job, /test:desktop:all|live-overlay\.spec\.js/);

  assert.match(requiredConfig, /forbidPending:\s*true/);
  assert.match(requiredConfig, /bail:\s*1/);
  assert.match(requiredConfig, /smoke\.spec\.js/);
  assert.doesNotMatch(requiredConfig, /live-overlay\.spec\.js/);
});

test("NSIS smoke is separate, install-based, and release-gated on provenance", async () => {
  const workflow = normalizeNewlines(await readRepoFile(".github/workflows/nsis-smoke.yml"));
  const smokeScript = await readRepoFile("desktop/tests/scripts/smoke-nsis.ps1");

  assert.match(workflow, /workflow_dispatch:/);
  assert.match(workflow, /schedule:/);
  assert.match(workflow, /release:\n\s+types:\s*\[published\]/);
  assert.match(workflow, /pnpm tauri build --bundles nsis/);
  assert.match(workflow, /smoke-nsis\.ps1/);
  assert.match(workflow, /assert-third-party-provenance\.mjs/);
  assert.match(workflow, /github\.event_name == 'release'/);
  assert.match(workflow, /actions\/upload-artifact@v\d+/);

  assert.match(smokeScript, /"\/S"/);
  assert.match(smokeScript, /"\/D=\$installRoot"/);
  assert.match(smokeScript, /THIRD_PARTY_NOTICES\.md/);
  assert.match(smokeScript, /Get-FileHash/);
  assert.match(smokeScript, /uninstall\.exe/);
});

test("Tauri packages the reviewed notice at a stable installed path", async () => {
  const tauriConfig = JSON.parse(await readRepoFile("desktop/src-tauri/tauri.conf.json"));
  const notice = await readRepoFile("THIRD_PARTY_NOTICES.md");

  assert.equal(
    tauriConfig.bundle.resources?.["../../THIRD_PARTY_NOTICES.md"],
    "THIRD_PARTY_NOTICES.md",
  );
  assert.match(notice, /^## FreeFlow$/m);
  assert.match(notice, /https:\/\/github\.com\/zachlatta\/freeflow/);
  assert.match(notice, /^MIT License$/m);
});
