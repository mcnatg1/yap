import assert from "node:assert/strict";

import { reviewedActionUses } from "./workflow-policy.mjs";

export function assertReviewedActionPins(workflow, workflowPath) {
  for (const [jobName, job] of Object.entries(workflow.jobs ?? {})) {
    if (job.uses) {
      assertReviewedUse(job.uses, `${workflowPath} ${jobName} reusable workflow`);
    }
    for (const step of job.steps ?? []) {
      if (step.uses) assertReviewedUse(step.uses, `${workflowPath} ${jobName} action`);
    }
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
