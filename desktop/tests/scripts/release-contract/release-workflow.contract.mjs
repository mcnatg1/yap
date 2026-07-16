import assert from "node:assert/strict";
import { access } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import {
  namedStepIndex,
  readWorkflow,
  repoRoot,
  workflowSteps,
} from "./workflow-access.mjs";
import { releaseActionUses, reviewedActions } from "./workflow-policy.mjs";

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
  assert.match(buildSteps[captureEnvironment].run, /Join-Path \$nsisRoot "makensis\.exe"/);
  assert.match(buildSteps[captureEnvironment].run, /\$nsisLauncher \/VERSION/);
  assert.match(buildSteps[captureEnvironment].run, /\$nsisCompiler \/VERSION/);
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

  assert.equal(
    publishSteps.some((step) => String(step.uses ?? "").startsWith("actions/checkout@")),
    false,
  );
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
