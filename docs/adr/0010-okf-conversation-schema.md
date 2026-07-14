# ADR 0010: OKF conversation schema

**Date:** 2026-06-30
**Status:** Accepted Markdown/YAML and raw-preservation principles; canonical Phase 9 schema pending
**Builds on:** [ADR 0004](0004-background-diarization-okf-agents.md) (OKF dirs, Archivist), [ADR 0009](0009-knowledge-worker-protocol.md) (worker writes these)
**Amended by:** [ADR 0017](0017-knowledge-base-compiler.md) — in the **team profile**, conversations enter **Lane 1** (content-addressed append store) rather than being written directly to OKF by the client worker; the **KB compiler** in `yap-server` normalises Lane 1 captures to OKF markdown and commits curated/stitched conversations to `yap-knowledge` (Lane 2).
**Amended by:** [ADR 0020](0020-meeting-capture-diarization-authority.md) — the example's fixed `SPEAKER_XX`, `source`, vault, and Opus assumptions are historical. Before implementation, the schema must represent session mode/origin separately from physical tracks, accept WAV or a later negotiated transport codec, and render revisioned `Unknown` / `Speaker N` or fully provenanced server identities. The general Markdown/YAML and raw/polished principles remain accepted; the sample is not an implementation-ready v1 contract.
**Amended by:** [ADR 0022](0022-google-okf-permission-safe-projections.md) — the pinned Google OKF v0.1 draft is the canonical Phase 9 base format. Yap adds a compatible enterprise profile for stable resource identity, typed relationships, provenance, and permission-safe compiled projections. The schema below remains historical.

## Context

ADR 0004 named the OKF directories (`conversations/`, `jargon_glossary/`, `work_artifacts/`, …) and said "Markdown + YAML frontmatter" but gave no concrete file schema. The Archivist (worker) and any reader (history UI, Librarian, MCP) need a pinned format. This ADR recorded an initial schema sketch; ADR 0017 and ADR 0020 later made its storage lane, source, codec, and speaker fields incomplete. ADR 0022 replaces the pending base schema with pinned Google OKF v0.1 plus a compatible Yap Enterprise OKF profile.

## Accepted principles

Conversation outputs remain **Markdown + YAML frontmatter**, UTF-8, human-readable, and git-friendly. Raw text is never discarded when a polished representation exists. ADR 0022 governs OKF conformance, stable resource identity, relationships, and projections; ADR 0017 and ADR 0020 govern storage lanes, source/track fields, audio references, and speaker assertions.

## Historical schema sketch (non-normative)

The remainder of this section preserves the original solo-profile proposal. It is decision history, not an implementation-ready v1 contract. That proposal used `%LOCALAPPDATA%/Yap/knowledge_base/`; any implementation now resolves the equivalent tree through Tauri app data (`%APPDATA%/com.mcnatg1.yap/knowledge_base/` on Windows).

### Conversation — `conversations/<session-id>.md`

```markdown
---
id: 2026-06-30T14-12-session-uuid
type: conversation
source: live | batch
audio: media_cache/<session-id>.opus
language: en
started_at: 2026-06-30T14:12:03-05:00
duration_ms: 842000
speakers:
  - id: SPEAKER_01
    name: null            # user may rename later
  - id: SPEAKER_02
    name: "Alex"
tags: []
degraded: false           # true if labels finished post-session
schema: 1
---

## Transcript

[00:00:03] **SPEAKER_01:** raw or polished text for this segment...
[00:00:11] **SPEAKER_02:** ...
```

- Body segments: `[hh:mm:ss] **SPEAKER_XX:**` + text; timestamps from alignment.
- Two text tracks: default body is **polished**; `transcript_raw` stored as a sibling fenced block or `<session-id>.raw.md` so raw is never lost ([ADR 0004](0004-background-diarization-okf-agents.md) dual-track).
- `speakers[].name` is display-only metadata; vault math keys on `id`.

### Glossary card — `jargon_glossary/<term-slug>.md`

```markdown
---
type: term
term: "OKF"
aliases: ["Open Knowledge Format"]
created: 2026-06-30
source_sessions: [<session-id>]
schema: 1
---

Open Knowledge Format — local markdown+YAML knowledge store...
See also: [[Speaker Vault]]
```

In the historical sketch, wiki-links `[[Term]]` were optional and resolved by the Curator under the old Phase 7d alias.

### Work artifact — `work_artifacts/<session-id>-todos.md`

```markdown
---
type: action_items
session: <session-id>
schema: 1
---

- [ ] (proposed, conf 0.62) Follow up with Alex on budget
- [ ] (todo, conf 0.91) Send notes by Friday
```

Coordinator distinguishes **proposed** vs **todo** by confidence ([ADR 0004 §8](0004-background-diarization-okf-agents.md)).

### Conventions
- `schema:` integer on every file; bump on breaking change, readers tolerate older.
- Filenames: ISO-ish session id / term slug; no spaces.
- The historical target had Yap **Transcripts history** mirror `conversations/` (or a subset). Current history does not read this schema.

## Consequences

### Positive
- A future pinned format gives the Archivist/compiler and history/Librarian/MCP readers one contract.
- Markdown+YAML = portable, diffable, openable in any editor or Obsidian-style tool.
- Dual-track raw preserved; renames don't corrupt vault.

### Negative
- Markdown parsing for retrieval is looser than a DB; mitigated by frontmatter + the index ([ADR 0011](0011-vector-rag-retrieval.md)).
- Schema migrations need care; the canonical Phase 9 schema must define versioning and compatibility before implementation.

### Neutral
- Historical alias Phase 7c is replaced by canonical Phase 9. Current Yap history does not append speaker tags from this schema.

## Alternatives considered
- **SQLite as the solo-profile content source of truth** — rejected: it hurts portability and inspection. ADR 0017 separately governs team Lane 1 storage and compiled indexes.
- **JSON conversations** — rejected: less human-readable than markdown for a notes product.
- **One big file per day** — rejected: per-session files diff and link better.

## References
- [ADR 0004](0004-background-diarization-okf-agents.md) — directories, Archivist, dual-track
- [ADR 0009](0009-knowledge-worker-protocol.md) — worker emits `session_stitched` with the path
- [ADR 0011](0011-vector-rag-retrieval.md) — indexing these files
