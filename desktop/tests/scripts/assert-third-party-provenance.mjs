import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..");
const manifestPath = path.join(repoRoot, "THIRD_PARTY_PROVENANCE.json");
const requireReviewed = process.argv.includes("--require-reviewed");
const unknownArgs = process.argv.slice(2).filter((arg) => arg !== "--require-reviewed");

if (unknownArgs.length > 0) {
  throw new Error(`Unknown provenance arguments: ${unknownArgs.join(", ")}`);
}

const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
assert(manifest.schemaVersion === 2, "Unsupported third-party provenance schema.");
assert(Array.isArray(manifest.sources) && manifest.sources.length > 0, "Provenance has no sources.");

const sourceIds = new Set();
const unreviewed = [];
for (const source of manifest.sources) {
  assert(typeof source.id === "string" && /^[a-z0-9-]+$/.test(source.id), "Invalid source ID.");
  assert(!sourceIds.has(source.id), `Duplicate provenance source ID: ${source.id}`);
  sourceIds.add(source.id);
  assert(isHttpsUrl(source.repository), `Source ${source.id} has an invalid repository URL.`);
  assert(typeof source.license === "string" && source.license.length > 0, `Source ${source.id} has no license.`);

  const revision = source.revision;
  assert(revision && ["unverified", "reviewed"].includes(revision.status), `Source ${source.id} has an invalid revision status.`);
  if (revision.status === "unverified") {
    assert(revision.value === null, `Unverified source ${source.id} must not claim a revision.`);
    unreviewed.push(source.id);
  } else {
    assertReviewedRevision(source);
  }

  await readRepoPath(source.notice, `notice for ${source.id}`);
  assert(Array.isArray(source.files) && source.files.length > 0, `Source ${source.id} has no file evidence.`);
  const filePaths = new Set();
  for (const file of source.files) {
    assert(!filePaths.has(file.path), `Source ${source.id} repeats file evidence for ${file.path}.`);
    filePaths.add(file.path);
    assert(/^[0-9a-f]{64}$/.test(file.sha256), `Source ${source.id} has an invalid SHA-256.`);
    const contents = await readRepoPath(file.path, `source evidence for ${source.id}`);
    const actual = createHash("sha256").update(contents).digest("hex");
    assert(actual === file.sha256, `Provenance hash mismatch for ${file.path}.`);
  }
}

if (requireReviewed && unreviewed.length > 0) {
  console.error(
    `Pre-publication provenance failed: no exact reviewed revision for ${unreviewed.join(", ")}.`,
  );
  process.exitCode = 1;
} else {
  const qualifier = unreviewed.length > 0
    ? `; unreviewed revisions: ${unreviewed.join(", ")}`
    : "; all revisions have exact review evidence";
  console.log(`Third-party provenance integrity passed${qualifier}.`);
}

export function assertReviewedRevision(source) {
  const revision = source.revision;
  assert(/^[0-9a-f]{40}$/.test(revision.value), `Reviewed source ${source.id} must record a full Git revision.`);
  const evidence = revision.evidence;
  assert(evidence && typeof evidence === "object", `Reviewed source ${source.id} has no review evidence.`);
  assert(
    evidence.commitUrl === `${source.repository}/commit/${revision.value}`,
    `Reviewed source ${source.id} does not bind its evidence to the recorded revision.`,
  );
  assert(evidence.licensePath === "LICENSE", `Reviewed source ${source.id} has no reviewed LICENSE path.`);
  assert(
    /^[0-9a-f]{64}$/.test(evidence.licenseSha256),
    `Reviewed source ${source.id} has no reviewed LICENSE SHA-256.`,
  );
  assert(
    evidence.reviewScope === "upstream revision and license attribution",
    `Reviewed source ${source.id} overstates or omits its review scope.`,
  );
  assert(
    evidence.localFileEvidence === "integrity-only",
    `Reviewed source ${source.id} must describe local hashes as integrity-only evidence.`,
  );
}

async function readRepoPath(relativePath, label) {
  assert(typeof relativePath === "string" && relativePath.length > 0, `Missing ${label} path.`);
  const absolutePath = path.resolve(repoRoot, relativePath);
  const relative = path.relative(repoRoot, absolutePath);
  assert(
    relative !== "" && !relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative),
    `${label} escapes the repository root.`,
  );
  return readFile(absolutePath);
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function isHttpsUrl(value) {
  try {
    return new URL(value).protocol === "https:";
  } catch {
    return false;
  }
}
