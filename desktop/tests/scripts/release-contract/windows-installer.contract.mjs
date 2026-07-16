import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { access } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import { readRepoFile, readWorkflow, repoRoot } from "./workflow-access.mjs";

test("NSIS uses stock Tauri behavior inside a disposable Windows boundary", async () => {
  const config = JSON.parse(await readRepoFile("desktop/src-tauri/tauri.conf.json"));
  const paths = await readRepoFile("desktop/src-tauri/src/paths.rs");
  const migration = await readRepoFile("desktop/src-tauri/src/paths/legacy_migration.rs");
  const migrationPlatform = await readRepoFile(
    "desktop/src-tauri/src/paths/legacy_migration/platform.rs",
  );
  const migrationRecovery = await readRepoFile(
    "desktop/src-tauri/src/paths/legacy_migration/recovery.rs",
  );
  const migrationTree = await readRepoFile(
    "desktop/src-tauri/src/paths/legacy_migration/secure_tree.rs",
  );
  const app = await readRepoFile("desktop/src-tauri/src/app.rs");
  const smoke = await readRepoFile("desktop/tests/scripts/smoke-nsis.ps1");
  assert.equal(config.identifier, "com.mcnatg1.yap");
  assert.match(paths, new RegExp(`PRODUCTION_IDENTIFIER: &str = "${config.identifier}"`));
  assert.equal(config.bundle.windows?.nsis?.installerHooks, undefined);
  assert.equal(config.bundle.windows?.nsis?.installMode, "currentUser");
  assert.deepEqual(config.bundle.windows?.webviewInstallMode, {
    type: "offlineInstaller",
    silent: true,
  });
  assert.match(migration, /\.legacy-migration\.lock/);
  assert.match(migration, /MIGRATION_LOCK_TIMEOUT/);
  assert.match(migration, /try_lock\(\)/);
  assert.match(migration, /MIGRATION_COMPLETION_FILE/);
  assert.match(migrationRecovery, /recover_migration_residue/);
  assert.match(migrationTree, /copy_tree_verified/);
  assert.match(migrationTree, /output\.sync_all\(\)/);
  assert.match(migrationTree, /trees_equal/);
  assert.match(migrationPlatform, /rename_no_replace/);
  assert.doesNotMatch(migrationPlatform, /MOVEFILE_REPLACE_EXISTING/);
  assert.match(app, /MessageBoxW/);
  assert.match(app, /Yap-startup-migration-error/);
  assert.doesNotMatch(JSON.stringify(config), /installerHooks|nsis-hooks\.nsh/);
  assert.match(smoke, /GITHUB_ACTIONS/);
  assert.match(smoke, /RUNNER_ENVIRONMENT/);
  assert.match(smoke, /github-hosted/);
  assert.match(smoke, /YAP_DISPOSABLE_WINDOWS/);
  assert.match(smoke, /com\.mcnatg1\.yap/);
  assert.match(smoke, /ApplicationData/);
  assert.match(smoke, /LocalApplicationData/);
  assert.match(smoke, /expectedInstallLocation/);
  assert.match(smoke, /Start-Process/);
  assert.match(smoke, /WaitForExit/);
  assert.match(smoke, /Kill\(\$true\)/);
  assert.match(smoke, /function Wait-ForPathAbsence[\s\S]*?Start-Sleep -Milliseconds 100/);
  assert.match(smoke, /-LiteralPaths @\(\$uninstallRegistryPath, \$installLocation\)/);
  assert.match(smoke, /ExpectedInstallerSha256/);
  assert.match(smoke, /THIRD_PARTY_NOTICES\.md/);
  assert.match(smoke, /THIRD_PARTY_PROVENANCE\.json/);
  assert.match(smoke, /stockSilentUninstallPreservedProductRegistry/);
  assert.match(smoke, /@\("\/S"\)/);
  assert.match(smoke, /preserved/i);
  assert.doesNotMatch(smoke, /DELETEAPPDATA|RMDir|Remove-Item|YAP_APP_DATA_DIR/);
  for (const retiredPath of [
    "desktop/src-tauri/nsis-hooks.nsh",
    "desktop/src-tauri/tauri.test.conf.json",
    "desktop/tests/scripts/build-nsis-test.ps1",
    "desktop/tests/scripts/nsis-smoke-helpers.psm1",
    "desktop/tests/scripts/nsis-smoke-helpers.test.ps1",
    "desktop/tests/scripts/smoke-nsis-local.ps1",
    "desktop/tests/scripts/smoke-nsis-production-delete.ps1",
    "desktop/tests/scripts/smoke-nsis-test-delete.ps1",
  ]) {
    await assert.rejects(access(path.join(repoRoot, retiredPath)), /ENOENT/);
  }
});

test("Windows release automation requires PowerShell 7.4 Core", async () => {
  const powerShellFiles = [
    "desktop/tests/scripts/bind-pnpm-cache-store.ps1",
    "desktop/tests/scripts/native-window-recovery.test.ps1",
    "desktop/tests/scripts/smoke-nsis.ps1",
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
  const runtimeRequirement = /^#requires -Version 7\.4\r?\n#requires -PSEdition Core\b/i;

  for (const relativePath of powerShellFiles) {
    const source = await readRepoFile(relativePath);
    assert.match(
      source,
      runtimeRequirement,
      `${relativePath} must fail fast outside PowerShell Core 7.4 or newer`,
    );
  }

  const legacyWindowsPowerShell = ["power", "shell.exe"].join("");
  const runtimeSelectors = [
    "desktop/package.json",
    "desktop/tests/scripts/release-contract/windows-installer.contract.mjs",
    "desktop/tests/wdio/live-overlay.spec.js",
    "desktop/tests/wdio/live-overlay-window-fixture.js",
  ];
  for (const relativePath of runtimeSelectors) {
    const source = (await readRepoFile(relativePath)).toLowerCase();
    assert.equal(
      source.includes(legacyWindowsPowerShell),
      false,
      `${relativePath} still selects legacy Windows PowerShell`,
    );
  }
  const liveOverlayFixture = await readRepoFile(
    "desktop/tests/wdio/live-overlay-window-fixture.js",
  );
  assert.match(liveOverlayFixture, /execFileAsync\(\s*"pwsh\.exe"/);

  const packageJson = JSON.parse(await readRepoFile("desktop/package.json"));
  assert.match(packageJson.scripts["test:nsis:disposable"], /(?:^|\s)pwsh\.exe\s/i);

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
      const guard = job.steps?.find((step) => step.name === "Verify PowerShell 7.4 Core");
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
      assert.match(guard.run, /\$PSVersionTable\.PSEdition\s+-cne\s+["']Core["']/);
      assert.match(guard.run, /\$PSVersionTable\.PSVersion\s+-lt\s+\[version\]["']7\.4["']/);
      assert.match(guard.run, /\bthrow\b/);
      assert.equal(
        job.steps.some((step) => /^powershell(?:\.exe)?$/i.test(String(step.shell ?? ""))),
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
  assert.match(runCompatibilitySuite.run, /Language\.Parser/);
  assert.match(runCompatibilitySuite.run, /PSEdition -cne "Core"/);
  assert.match(runCompatibilitySuite.run, /PSVersion\.ToString\(\)/);
  assert.match(runCompatibilitySuite.run, /POWERSHELL_74_VERSION/);
  assert.doesNotMatch(runCompatibilitySuite.run, /nsis-smoke-helpers/);

  const smokeScriptPath = path
    .join(repoRoot, "desktop/tests/scripts/smoke-nsis.ps1")
    .replaceAll("'", "''");
  const legacyResult = spawnSync(
    legacyWindowsPowerShell,
    ["-NoProfile", "-NonInteractive", "-File", smokeScriptPath],
    { cwd: repoRoot, encoding: "utf8", timeout: 10_000 },
  );
  assert.notEqual(
    legacyResult.status,
    0,
    "legacy Windows PowerShell unexpectedly ran release automation",
  );
  assert.match(
    `${legacyResult.stdout}\n${legacyResult.stderr}`,
    /#requires[\s\S]*PowerShell 7\.4|PSEdition Core/i,
  );
});
