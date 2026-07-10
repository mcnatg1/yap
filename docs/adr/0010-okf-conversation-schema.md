# ADR 0010: OKF conversation schema

**Date:** 2026-06-30
**Status:** Accepted schema (canonical Phase 9)
**Builds on:** [ADR 0004](0004-background-diarization-okf-agents.md) (OKF dirs, Archivist), [ADR 0009](0009-knowledge-worker-protocol.md) (worker writes these)
**Amended by:** [ADR 0017](0017-knowledge-base-compiler.md) — in the **team profile**, conversations enter **Lane 1** (content-addressed append store) rather than being written directly to OKF by the client worker; the **KB compiler** in `yap-server` normalises Lane 1 captures to OKF markdown and commits curated/stitched conversations to `yap-knowledge` (Lane 2). The **file schema** (frontmatter fields, markdown body format, dual-track raw/polished) defined in this ADR is **unchanged** — it is the output format of the KB compiler in both profiles.

## Context

ADR 0004 named the OKF directories (`conversations/`, `jargon_glossary/`, `work_artifacts/`, …) and said "Markdown + YAML frontmatter" but gave no concrete file schema. The Archivist (worker) and any reader (history UI, Librarian, MCP) need a pinned format. This ADR defines the **v1 file schemas**; agents/wiki-links beyond v1 are noted as optional.

## Decision

Files are **Markdown + YAML frontmatter**, UTF-8, human-readable, git-friendly. Under `%LOCALAPPDATA%/Yap/knowledge_base/`.

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

Wiki-links `[[Term]]` are **optional in v1**; resolved by the Curator (Phase 7d).

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
- Yap **Transcripts history** mirrors `conversations/` (or a subset) and can render these directly before the full agent loop ships.

## Consequences

### Positive
- One pinned format for Archivist writer + history/Librarian/MCP readers.
- Markdown+YAML = portable, diffable, openable in any editor or Obsidian-style tool.
- Dual-track raw preserved; renames don't corrupt vault.

### Negative
- Markdown parsing for retrieval is looser than a DB; mitigated by frontmatter + the index ([ADR 0011](0011-vector-rag-retrieval.md)).
- Schema migrations need care (handled by `schema:` field).

### Neutral
- Phase 7c; until then speaker tags append to existing Yap history JSON ([ADR 0004 §10](0004-background-diarization-okf-agents.md)).

## Alternatives considered
- **SQLite as source of truth** — rejected for content: hurts local-first portability/inspection; SQLite is used only for the **search index** ([ADR 0011](0011-vector-rag-retrieval.md)), not canonical storage.
- **JSON conversations** — rejected: less human-readable than markdown for a notes product.
- **One big file per day** — rejected: per-session files diff and link better.

## References
- [ADR 0004](0004-background-diarization-okf-agents.md) — directories, Archivist, dual-track
- [ADR 0009](0009-knowledge-worker-protocol.md) — worker emits `session_stitched` with the path
- [ADR 0011](0011-vector-rag-retrieval.md) — indexing these files
