# Public Security Posture

This document describes implemented controls and explicit handoffs without
publishing private security evidence. It is not a penetration-test report,
certification, production authorization, or substitute for enterprise review.

## Implemented Phase 1–5 controls

### Local data and filesystem

- Tauri's canonical app-data directory owns runtime data. Legacy migration is
  serialized, bounded, non-following, conflict-aware, and hash-verifies staged
  and destination data before source retirement.
- External recordings are admitted through native picker/drop and path identity
  checks. Cancel, retry, retention, and cleanup do not delete source media.
- Private files use owner-controlled locations and atomic publication. Reads
  and mutations reject unexpected file types, links/reparse points, path escape,
  replacement identity, size/extent mismatch, and invalid hash/schema lineage.
- Destructive recording/result operations use explicit intent, quarantine, and
  revalidation. Recovery preserves ambiguous evidence rather than guessing.
- Renderer playback and transcript actions require native admission/authorization;
  a path string alone is not capability authority.

### Runtime and process

- Audio callback work is preallocated/non-blocking and queues/resources are
  bounded. Loss and worker failures become explicit state instead of fabricated
  successful audio.
- Native background work has explicit lifecycle ownership and shutdown paths.
- The server reference worker runs non-root, without network, with read-only and
  bounded mounts/resources, dropped privileges/capabilities, immutable
  image/model identity, bounded output, and unconditional cleanup.
- Installer-only custom containment has been retired. Stock Tauri NSIS behavior
  is tested in a disposable Windows environment; genuine runtime process safety
  remains in product/server code.

### Network and protocol

- The current application server binds to numeric loopback. The development
  private-node path uses an explicitly managed SSH local forward and no
  application-controlled alias failover.
- Desktop configuration validates/approves origins and binds in-flight work to
  a configuration generation. Stale-origin responses cannot mutate current
  job state.
- HTTP requests/responses, headers, bodies, chunks, files, jobs, retries,
  workers, queues, durations, retention, and transcript/model metadata are
  bounded and contract-validated.
- Create/upload/commit/cancel/result behavior is idempotent or conflict-visible;
  server result identity/hashes/authority are reverified before native History
  publication.
- Health advertises capability only when the runtime is ready. Unsupported live
  transport remains unavailable rather than being presented as healthy.

### UI and local control

- Tauri command authorization is window-aware and domain owners validate
  untrusted invoke data before mutation.
- One native tray/island owner controls window bounds and the visible hit region;
  no duplicate invisible window catches clicks.
- Shortcut enrollment is deliberate and bounded so ordinary typing is not
  captured as configuration.
- User-visible errors use stable state/codes and avoid private audio/transcript
  content. Private diagnostic and scan material stays outside Git/PR/hosted logs.

### Supply chain and release

- Node, pnpm, Rust, Python, container, model, and critical tool/action identities
  are constrained by manifests, lockfiles, hashes, reviewed revisions, or
  immutable digests as appropriate.
- Directly adapted third-party source has a pinned upstream revision, verified
  license, local file hashes, notice, and an executable provenance contract.
- Release contracts bind cache use, build inputs, artifact hash, evidence, and
  immutable commit identity. The staged release workflow creates a draft only
  from the verified commit/artifact transaction.
- Focused tests run during development; exact-head phase/checkpoint gates and
  hosted PR checks precede merge.

## Known boundaries, not hidden controls

The current loopback/SSH development profile does not provide:

- Entra/MSAL authentication or Yap API token validation;
- tenant-derived `(tid, oid)` ownership, authorization, revocation, or purpose grants;
- an external TLS endpoint, enterprise certificate, internal DNS, or app-owned WSS;
- an IT-approved firewall policy or ZPA application segment;
- persistent production service supervision, backup/restore, disaster recovery,
  monitoring/SIEM integration, or measured multi-user capacity; or
- enterprise deployment/publication approval.

These are accepted Phase 7/10 and IT/security/network handoffs in the
[roadmap](../roadmap/ROADMAP.md). Developer-owned infrastructure must not be
described as satisfying them.

## Security review and disclosure handling

- Correctness and security findings must be resolved before checkpoint/phase
  merge or recorded as an explicit later owner/handoff.
- Private scans, scan identifiers, exploit details, machine paths, private
  audio/transcripts, and raw host evidence are never committed or summarized in
  public PR/CI output.
- A full Codex security plugin scan is intentionally deferred until the accepted
  Phase 10 enterprise gate. Normal development still requires focused threat
  reasoning, code review, tests, dependency audits, and safe design.
- If a vulnerability is suspected, stop publication, preserve private evidence,
  fix and validate the affected boundary, and disclose only through the
  repository owner's approved private channel.
