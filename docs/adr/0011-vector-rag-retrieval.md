# ADR 0011: Vector index + RAG retrieval (L6–L7)

**Date:** 2026-06-30
**Status:** Accepted (roadmap — Phase 7e)
**Builds on:** [ADR 0010](0010-okf-conversation-schema.md) (what gets indexed), [ADR 0005](0005-llama-server-agents.md) (Analyst LLM), [ADR 0006](0006-silero-agents-state-machine.md) (Librarian/Analyst profiles)
**Amended by:** [ADR 0017](0017-knowledge-base-compiler.md) — in the **team profile**, the local SQLite + `sqlite-vec` index is **replaced by a server-side vector DB** (Milvus/pgvector-class) as a compiled disposable layer in `yap-server`. The vector schema gains `permission_hash`, `access_tags`, `repo_commit`/`content_hash` fields to support permission-filtered retrieval. The retrieval flow, reciprocal-rank fusion, and confidence gate (< 0.60 → refuse) are **preserved**. The **solo/local-first profile** retains the local SQLite approach defined in this ADR.

## Context

The pipeline charts show "vector search" (L6) and Librarian→Analyst Q&A (L7), but no embedding model, index format, chunk strategy, or retrieval flow. This ADR pins them so "ask your knowledge base" is buildable. Must stay **local-first, CPU-friendly**, and keep OKF markdown ([ADR 0010](0010-okf-conversation-schema.md)) as the source of truth — the index is derived/disposable.

## Decision

### Index store

| Item | Decision |
|------|----------|
| Store | **SQLite** at `knowledge_base/.index/kb.sqlite` (derived; rebuildable from markdown) |
| Lexical | **FTS5** full-text table (BM25) |
| Vector | **`sqlite-vec`** extension for cosine KNN |
| Why | One embedded file, no server, hybrid in one place; matches local-first |

### Embeddings

| Item | Decision |
|------|----------|
| Model | `bge-small-en-v1.5` ONNX (~130 MB), CPU via `ort` |
| Dim | 384 |
| Where | Computed by the **knowledge worker** (already has ORT) on `chunk_done`/`session_stitched` |
| Delivery | On-demand download to `YAP_MODELS_DIR` when KB Q&A first enabled |

English-first (matches Live EN scope); multilingual embed model is a later swap behind the same interface.

### Chunking for retrieval
- Unit = **conversation segment / paragraph** from the OKF body (already speaker- and time-bounded), not arbitrary token windows.
- Store per chunk: `session_id`, `t0_ms`, `speaker_id`, `text`, `embedding`, `source_path`.
- Re-index incrementally per session; full rebuild = delete `.index/` and replay markdown.

### Retrieval flow (Librarian → Analyst)

```
query → embed → vector KNN (top 20) ∪ FTS5 BM25 (top 20)
      → reciprocal-rank fusion → top K (default 8)
      → confidence = fused score of best hit
```

| Gate ([ADR 0004 §8](0004-background-diarization-okf-agents.md)) | Behavior |
|------|----------|
| best score < **0.60** | Librarian refuses; Analyst returns "no solid notes" template — **no hallucinated answer** |
| > 50 candidate hits | pass **K most recent**; offer "summarize older?" |
| ok | Analyst answers from context pack, **citations required** (`[session, timestamp]`) |

Librarian uses **no LLM** (retrieval only); Analyst uses llama-server (`llm_background`/`INTERACTIVE`).

## Consequences

### Positive
- Single SQLite file; hybrid lexical+vector; no extra service.
- Markdown stays canonical; index is disposable/rebuildable.
- Confidence floor + mandatory citations curb RAG hallucination.

### Negative
- `sqlite-vec` + ONNX embeddings are extra native deps (shared `ort` with worker helps).
- Re-embedding cost on large histories (mitigated: incremental per session, BELOW_NORMAL).

### Neutral
- Phase 7e; English-first.

## Alternatives considered
- **LanceDB / Chroma / Qdrant** — rejected for v1: heavier deps/servers vs one SQLite file.
- **Pure BM25, no vectors** — rejected: misses paraphrase/semantic recall.
- **llama.cpp embedding endpoint** — deferred: viable, but a dedicated small ONNX embed model is lighter and decouples from the chat model.
- **Cloud embeddings/RAG** — rejected: violates local-first.

## References
- [ADR 0010](0010-okf-conversation-schema.md) — indexed content
- [ADR 0012](0012-mcp-server-surface.md) — exposes search to external tools
