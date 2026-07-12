import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const defaultRepoRoot = path.resolve(import.meta.dirname, "..", "..", "..");
const MAX_UPSTREAM_EVIDENCE_BYTES = 1024 * 1024;

export async function verifyProvenance({
  fetchImpl = globalThis.fetch,
  repoRoot = defaultRepoRoot,
  requireReviewed = false,
  verifyUpstream = false,
} = {}) {
  const manifestPath = path.join(repoRoot, "THIRD_PARTY_PROVENANCE.json");
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  assert(manifest.schemaVersion === 2, "Unsupported third-party provenance schema.");
  assert(Array.isArray(manifest.sources) && manifest.sources.length > 0, "Provenance has no sources.");

  const sourceIds = new Set();
  const reviewed = [];
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

    const noticeBytes = await readRepoPath(repoRoot, source.notice, `notice for ${source.id}`);
    if (revision.status === "reviewed") reviewed.push({ noticeBytes, source });
    assert(Array.isArray(source.files) && source.files.length > 0, `Source ${source.id} has no file evidence.`);
    const filePaths = new Set();
    for (const file of source.files) {
      assert(!filePaths.has(file.path), `Source ${source.id} repeats file evidence for ${file.path}.`);
      filePaths.add(file.path);
      assert(/^[0-9a-f]{64}$/.test(file.sha256), `Source ${source.id} has an invalid SHA-256.`);
      const contents = await readRepoPath(repoRoot, file.path, `source evidence for ${source.id}`);
      const actual = createHash("sha256").update(contents).digest("hex");
      assert(actual === file.sha256, `Provenance hash mismatch for ${file.path}.`);
    }
  }

  if ((requireReviewed || verifyUpstream) && unreviewed.length > 0) {
    throw new Error(`Pre-publication provenance failed: no exact reviewed revision for ${unreviewed.join(", ")}.`);
  }
  if (verifyUpstream) {
    assert(typeof fetchImpl === "function", "Upstream provenance verification requires fetch.");
    for (const { noticeBytes, source } of reviewed) {
      await verifyReviewedSourceUpstream(source, { fetchImpl, noticeBytes });
    }
  }

  return { reviewed: reviewed.map(({ source }) => source.id), unreviewed };
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
  assert(
    Array.isArray(evidence.upstreamFiles) && evidence.upstreamFiles.length > 0,
    `Reviewed source ${source.id} has no upstream source-file evidence.`,
  );
  const upstreamPaths = new Set();
  for (const file of evidence.upstreamFiles) {
    assert(
      typeof file.path === "string"
      && file.path.length > 0
      && !path.isAbsolute(file.path)
      && !file.path.split(/[\\/]/).includes("..")
      && /^[A-Za-z0-9._/-]+$/.test(file.path),
      `Reviewed source ${source.id} has an unsafe upstream path.`,
    );
    assert(!upstreamPaths.has(file.path), `Reviewed source ${source.id} repeats upstream path ${file.path}.`);
    upstreamPaths.add(file.path);
    assert(/^[0-9a-f]{64}$/.test(file.sha256), `Reviewed source ${source.id} has an invalid upstream SHA-256.`);
  }
}

export async function verifyReviewedSourceUpstream(source, {
  fetchImpl = globalThis.fetch,
  noticeBytes,
  timeoutMs = 15_000,
} = {}) {
  assertReviewedRevision(source);
  assert(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, "Upstream verification timeout must be positive.");
  assert(typeof fetchImpl === "function", "Upstream provenance verification requires fetch.");

  const repository = parseGitHubRepository(source.repository, source.id);
  const revision = source.revision.value;
  const commitApiUrl = `https://api.github.com/repos/${repository.owner}/${repository.name}/commits/${revision}`;
  const licenseUrl = `https://raw.githubusercontent.com/${repository.owner}/${repository.name}/${revision}/${source.revision.evidence.licensePath}`;
  const apiHeaders = {
    accept: "application/vnd.github+json",
    "user-agent": "Yap-release-provenance",
    "x-github-api-version": "2022-11-28",
  };
  if (process.env.GITHUB_TOKEN) apiHeaders.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

  try {
    const commitBytes = await fetchBoundedBodyWithDeadline(fetchImpl, commitApiUrl, {
      headers: apiHeaders,
      label: `commit evidence for ${source.id}`,
      maxBytes: MAX_UPSTREAM_EVIDENCE_BYTES,
      timeoutMs,
    });
    let commit;
    try {
      commit = JSON.parse(commitBytes.toString("utf8"));
    } catch {
      throw new Error(`Upstream commit evidence for ${source.id} was not valid JSON.`);
    }
    if (commit?.sha !== revision) {
      throw new Error(`Upstream commit evidence for ${source.id} returned a different commit.`);
    }

    const licenseBytes = await fetchBoundedBodyWithDeadline(fetchImpl, licenseUrl, {
      headers: { "user-agent": "Yap-release-provenance" },
      label: `license evidence for ${source.id}`,
      maxBytes: MAX_UPSTREAM_EVIDENCE_BYTES,
      timeoutMs,
    });
    const licenseSha256 = createHash("sha256").update(licenseBytes).digest("hex");
    if (licenseSha256 !== source.revision.evidence.licenseSha256) {
      throw new Error(`Upstream license hash mismatch for ${source.id}.`);
    }
    if (noticeBytes !== undefined) {
      const notice = normalizeText(Buffer.from(noticeBytes).toString("utf8"));
      const license = normalizeText(licenseBytes.toString("utf8"));
      if (!notice.includes(license)) {
        throw new Error(`Shipped notice does not contain the reviewed license for ${source.id}.`);
      }
    }

    for (const file of source.revision.evidence.upstreamFiles) {
      const upstreamPath = file.path.split("/").map(encodeURIComponent).join("/");
      const fileUrl = `https://raw.githubusercontent.com/${repository.owner}/${repository.name}/${revision}/${upstreamPath}`;
      const fileBytes = await fetchBoundedBodyWithDeadline(fetchImpl, fileUrl, {
        headers: { "user-agent": "Yap-release-provenance" },
        label: `upstream source evidence ${file.path} for ${source.id}`,
        maxBytes: MAX_UPSTREAM_EVIDENCE_BYTES,
        timeoutMs,
      });
      const actual = createHash("sha256").update(fileBytes).digest("hex");
      if (actual !== file.sha256) {
        throw new Error(`Upstream source hash mismatch for ${source.id}: ${file.path}.`);
      }
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`Upstream verification failed for ${source.id}: ${message}`, { cause: error });
  }
}

function normalizeText(value) {
  return value.replace(/\r\n/g, "\n").trim();
}

async function fetchBoundedBodyWithDeadline(
  fetchImpl,
  url,
  { headers, label, maxBytes, timeoutMs },
) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(new Error(`request exceeded ${timeoutMs} ms`)), timeoutMs);
  try {
    const response = await fetchImpl(url, {
      headers,
      redirect: "follow",
      signal: controller.signal,
    });
    assertResponseOk(response, label);
    return await readBoundedBody(response, maxBytes);
  } finally {
    clearTimeout(timeout);
  }
}

async function readBoundedBody(response, maxBytes) {
  const declaredLength = response.headers?.get?.("content-length");
  if (declaredLength !== null && declaredLength !== undefined) {
    const parsed = Number(declaredLength);
    if (Number.isFinite(parsed) && parsed > maxBytes) {
      throw new Error(`Upstream evidence exceeds ${maxBytes} bytes.`);
    }
  }
  if (!response.body) return Buffer.alloc(0);

  const chunks = [];
  let total = 0;
  for await (const chunk of response.body) {
    const bytes = Buffer.from(chunk);
    total += bytes.length;
    if (total > maxBytes) throw new Error(`Upstream evidence exceeds ${maxBytes} bytes.`);
    chunks.push(bytes);
  }
  return Buffer.concat(chunks, total);
}

function assertResponseOk(response, label) {
  if (!response || response.ok !== true) {
    const status = Number.isInteger(response?.status) ? ` (${response.status})` : "";
    throw new Error(`${label} request failed${status}.`);
  }
}

function parseGitHubRepository(repositoryUrl, sourceId) {
  const url = new URL(repositoryUrl);
  const segments = url.pathname.split("/").filter(Boolean);
  assert(
    url.hostname === "github.com" && segments.length === 2,
    `Reviewed source ${sourceId} must use a canonical GitHub repository URL.`,
  );
  const [owner, name] = segments;
  assert(/^[A-Za-z0-9_.-]+$/.test(owner) && /^[A-Za-z0-9_.-]+$/.test(name), `Source ${sourceId} has an unsafe GitHub repository path.`);
  return { owner, name };
}

async function readRepoPath(repoRoot, relativePath, label) {
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

function parseArguments(args) {
  const allowed = new Set(["--require-reviewed", "--verify-upstream"]);
  const unknown = args.filter((arg) => !allowed.has(arg));
  if (unknown.length > 0) throw new Error(`Unknown provenance arguments: ${unknown.join(", ")}`);
  return {
    requireReviewed: args.includes("--require-reviewed"),
    verifyUpstream: args.includes("--verify-upstream"),
  };
}

async function main(args) {
  const options = parseArguments(args);
  const result = await verifyProvenance(options);
  const qualifier = result.unreviewed.length > 0
    ? `; unreviewed revisions: ${result.unreviewed.join(", ")}`
    : options.verifyUpstream
      ? "; all revisions and upstream license evidence verified"
      : "; all revisions have exact local review evidence";
  console.log(`Third-party provenance integrity passed${qualifier}.`);
}

const entryPoint = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : "";
if (entryPoint === import.meta.url) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
