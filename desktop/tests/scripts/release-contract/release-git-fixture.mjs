import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

export async function createReleaseGitFixture(prefix = "yap-release-contract-") {
  const fixtureRoot = await mkdtemp(path.join(os.tmpdir(), prefix));
  const files = {
    ".gitattributes": "*.nsh text\n",
    ".gitignore": ".env\n.env.*\n*.env\n/context.json\n/artifact-seal.json\n/metadata*.json\n/ambiguous.json\n/github-output.txt\n/bundle/\n/ambiguous/\n",
    "THIRD_PARTY_NOTICES.md": "fixture notices\n",
    "THIRD_PARTY_PROVENANCE.json": "{}\n",
    "desktop/package.json": `${JSON.stringify({ version: "0.1.0" })}\n`,
    "desktop/pnpm-lock.yaml": "lockfileVersion: '9.0'\n",
    "desktop/src/app.ts": "export const fixture = true;\n",
    "desktop/src-tauri/Cargo.lock": "# fixture lock\n",
    "desktop/src-tauri/Cargo.toml": "[package]\nname = 'fixture'\nversion = '0.1.0'\n",
    "desktop/src-tauri/rust-toolchain.toml": "[toolchain]\nchannel = '1.96.0'\n",
    "desktop/src-tauri/tauri.conf.json": `${JSON.stringify({ version: "0.1.0" })}\n`,
  };
  for (const [relativePath, contents] of Object.entries(files)) {
    const absolutePath = path.join(fixtureRoot, relativePath);
    await mkdir(path.dirname(absolutePath), { recursive: true });
    await writeFile(absolutePath, contents);
  }
  execFileSync("git", ["init", "-q"], { cwd: fixtureRoot });
  execFileSync("git", ["config", "user.email", "release-fixture@example.invalid"], {
    cwd: fixtureRoot,
  });
  execFileSync("git", ["config", "user.name", "Release Fixture"], { cwd: fixtureRoot });
  execFileSync("git", ["config", "core.autocrlf", "false"], { cwd: fixtureRoot });
  execFileSync("git", ["add", "."], { cwd: fixtureRoot });
  execFileSync("git", ["commit", "-q", "-m", "release fixture"], { cwd: fixtureRoot });
  const commitSha = execFileSync("git", ["rev-parse", "HEAD^{commit}"], {
    cwd: fixtureRoot,
    encoding: "utf8",
  }).trim();
  assert.equal(
    execFileSync("git", ["status", "--porcelain"], { cwd: fixtureRoot, encoding: "utf8" }).trim(),
    "",
  );
  return { commitSha, files, fixtureRoot };
}
