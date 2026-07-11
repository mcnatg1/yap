import { readFile } from "node:fs/promises";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..");
const noticePath = path.join(repoRoot, "THIRD_PARTY_NOTICES.md");
const notice = await readFile(noticePath, "utf8");
const freeFlowRevision = /^- Upstream revision:\s*`?([0-9a-f]{40})`?\s*$/im.exec(notice)?.[1];

if (!freeFlowRevision) {
  console.error(
    "Published releases require the exact 40-character FreeFlow upstream revision in "
      + "THIRD_PARTY_NOTICES.md. The local history does not establish that revision, so it "
      + "must be verified from the original import before release.",
  );
  process.exitCode = 1;
} else {
  console.log(`Verified FreeFlow upstream revision ${freeFlowRevision}.`);
}
