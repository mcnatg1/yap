import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import {
  assertReviewedRevision,
  verifyReviewedSourceUpstream,
} from "../assert-third-party-provenance.mjs";
import { readRepoFile, runNodeScript } from "./workflow-access.mjs";

function isGitHubApiUrl(value) {
  const url = new URL(String(value));
  return url.protocol === "https:"
    && url.hostname === "api.github.com"
    && url.port === "";
}

test("provenance gate requires exact scoped review evidence and current local hashes", async () => {
  const manifest = JSON.parse(await readRepoFile("THIRD_PARTY_PROVENANCE.json"));
  const tauriConfig = JSON.parse(await readRepoFile("desktop/src-tauri/tauri.conf.json"));
  const notice = await readRepoFile("THIRD_PARTY_NOTICES.md");
  const freeFlowZachLatta = manifest.sources.find(({ id }) => id === "freeflow-zachlatta");

  assert.equal(manifest.schemaVersion, 2);
  assert.equal(freeFlowZachLatta.repository, "https://github.com/zachlatta/freeflow");
  assert.equal(freeFlowZachLatta.license, "MIT");
  assert.equal(freeFlowZachLatta.revision.status, "reviewed");
  assert.equal(freeFlowZachLatta.revision.value, "7427ca982c19746770f5357ced16e993f2eb27fd");
  assert.equal(
    freeFlowZachLatta.revision.evidence.licenseSha256,
    "121e01b10b43ece3c10ce3eaf5db22915326aad843c3a271a834660096467add",
  );
  assert.equal(freeFlowZachLatta.revision.evidence.localFileEvidence, "integrity-only");
  assert.equal(freeFlowZachLatta.revision.evidence.upstreamFiles.length, 2);
  assert.equal(freeFlowZachLatta.notice, "THIRD_PARTY_NOTICES.md");
  assert.deepEqual(
    freeFlowZachLatta.files.map(({ path }) => path).sort(),
    [
      "desktop/src-tauri/src/audio/preprocess.rs",
      "desktop/src/components/live/live-overlay-views.tsx",
      "desktop/src/components/live/live-overlay.tsx",
      "desktop/src/components/live/live-waveform.tsx",
      "desktop/src/components/live/use-live-overlay-presentation.ts",
      "desktop/src/components/live/use-prefers-reduced-motion.ts",
    ],
  );
  for (const file of freeFlowZachLatta.files) {
    assert.match(file.sha256, /^[0-9a-f]{64}$/);
    assert.equal(typeof file.path, "string");
  }
  assert.match(notice, /^## FreeFlow \(zachlatta\/freeflow\)$/m);
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
  const arbitraryRevision = structuredClone(freeFlowZachLatta);
  arbitraryRevision.revision.value = "a".repeat(40);
  assert.throws(
    () => assertReviewedRevision(arbitraryRevision),
    /does not bind its evidence to the recorded revision/,
  );

  const fixtureSource = structuredClone(freeFlowZachLatta);
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
    if (isGitHubApiUrl(url)) {
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
      fetchImpl: async (url) => isGitHubApiUrl(url)
        ? new Response(JSON.stringify({ sha: "b".repeat(40) }), { status: 200 })
        : new Response(licenseBytes, { status: 200 }),
      timeoutMs: 1_000,
    }),
    /returned a different commit/i,
  );
  await assert.rejects(
    verifyReviewedSourceUpstream(fixtureSource, {
      fetchImpl: async (url) => isGitHubApiUrl(url)
        ? new Response(JSON.stringify({ sha: fixtureSource.revision.value }), { status: 200 })
        : new Response("wrong license", { status: 200 }),
      timeoutMs: 1_000,
    }),
    /license hash mismatch/i,
  );
});
