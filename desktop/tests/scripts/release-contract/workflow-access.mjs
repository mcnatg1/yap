import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";

const require = createRequire(import.meta.url);
const { parse: parseYaml } = require("yaml");

export const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..", "..");

export async function readRepoFile(relativePath) {
  return readFile(path.join(repoRoot, relativePath), "utf8");
}

export async function readWorkflow(relativePath) {
  return parseYaml(await readRepoFile(relativePath));
}

export async function discoveredWorkflowPaths() {
  const workflowsRoot = path.join(repoRoot, ".github", "workflows");
  const entries = await readdir(workflowsRoot, { withFileTypes: true });
  return entries
    .filter((entry) => entry.isFile() && /\.ya?ml$/i.test(entry.name))
    .map((entry) => `.github/workflows/${entry.name}`)
    .sort();
}

export function normalizedRunBody(source) {
  return String(source).replaceAll("\r\n", "\n").trim();
}

export function workflowSteps(workflow, jobName) {
  const job = workflow.jobs?.[jobName];
  assert.ok(job, `workflow is missing the ${jobName} job`);
  assert.ok(Array.isArray(job.steps), `${jobName} does not define steps`);
  return { job, steps: job.steps };
}

export function namedStepIndex(steps, name) {
  const index = steps.findIndex((step) => step.name === name);
  assert.notEqual(index, -1, `workflow is missing the ${name} step`);
  return index;
}

export function runNodeScript(relativePath, args = []) {
  return spawnSync(process.execPath, [path.join(repoRoot, relativePath), ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

export function workflowStepEntries(workflow) {
  return Object.entries(workflow.jobs ?? {}).flatMap(([jobName, job]) =>
    (job.steps ?? []).map((step, index) => ({ index, jobName, step })),
  );
}
