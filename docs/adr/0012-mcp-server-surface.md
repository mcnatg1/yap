# ADR 0012: MCP server surface

**Date:** 2026-06-30
**Status:** Accepted (roadmap — Phase 7e)
**Builds on:** [ADR 0011](0011-vector-rag-retrieval.md) (retrieval backend), [ADR 0010](0010-okf-conversation-schema.md) (data), [ADR 0006](0006-silero-agents-state-machine.md) (agent profiles)
**Amended by:** [ADR 0017](0017-knowledge-base-compiler.md) — in the **team profile**, the MCP server runs as a **sidecar of `yap-server`** (not the desktop app), exposing a compiled, permission-filtered KB view. The tool surface (`search_transcripts`, `get_conversation`, etc.) and read-mostly safety model are **preserved**. The **solo/local-first profile** retains the local stdio MCP server as specified in this ADR.

## Context

L6 lists "MCP" as an ecosystem gateway so external MCP clients can query the user's transcripts/knowledge base. No tools, resources, or transport were defined. This ADR pins a **minimal, read-mostly** MCP surface over the OKF + index.

## Decision

Yap ships an **MCP server** exposing the local knowledge base. Local-first: localhost/stdio only, no remote exposure, user opt-in in Settings.

### Transport
- **stdio** MCP server (`yap-mcp`) — primary, for IDE/desktop MCP clients.
- Optional localhost HTTP/SSE later; not v1.
- Reuses the [ADR 0011](0011-vector-rag-retrieval.md) SQLite index and [ADR 0010](0010-okf-conversation-schema.md) markdown (read-only by default).

### Tools (v1)

| Tool | Input | Output | LLM? |
|------|-------|--------|------|
| `search_transcripts` | `query`, `k?` | ranked snippets + `[session, t0_ms]` citations | No (Librarian retrieval) |
| `get_conversation` | `session_id` | full OKF markdown body | No |
| `list_glossary` | `prefix?` | term cards | No |
| `get_action_items` | `session_id?`, `status?` | todos/proposed | No |
| `define_term` *(write, opt-in)* | `term`, `definition` | created card path | No |

`search_transcripts` is the Librarian (retrieval only). Answer synthesis stays in Yap's **Analyst** (citations required) — MCP exposes **grounded data**, not an ungrounded chat endpoint.

### Resources
- `conversation://<session-id>` → markdown
- `glossary://<term-slug>` → card

### Safety / scope
- **Read-only by default**; the single write tool (`define_term`) is behind the same opt-in as Curator git ([ADR 0004 §10](0004-background-diarization-okf-agents.md)).
- No tool returns audio or file-system paths outside `knowledge_base/`.
- Server runs only while Yap (or an explicit `yap-mcp` invocation) runs; not a background daemon.

## Consequences

### Positive
- Transcripts become usable from external MCP clients without copy-paste.
- Thin layer over existing retrieval ([ADR 0011](0011-vector-rag-retrieval.md)); little new logic.
- Read-mostly + opt-in keeps the local-first trust model intact.

### Negative
- An MCP server is another surface to version/secure (mitigated: localhost, opt-in, read-mostly).
- Tool schema churn as MCP spec evolves; pin the SDK version.

### Neutral
- Phase 7e; gated behind KB + index existing.

## Alternatives considered
- **Full read/write MCP (edit transcripts, run agents)** — rejected v1: larger attack/During-edit surface; start read-mostly.
- **REST API instead of MCP** — rejected: MCP is the target integration (IDEs/agents); REST adds nothing for v1.
- **No gateway, in-app only** — rejected as long-term: external tool access is a stated L6 goal; deferring is fine, omitting isn't.

## References
- [ADR 0011](0011-vector-rag-retrieval.md) — search backend
- [ADR 0010](0010-okf-conversation-schema.md) — resource content
- [mcp-builder skill] for implementation when Phase 7e starts
