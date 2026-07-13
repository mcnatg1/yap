# ADR 0022: Google OKF and permission-safe knowledge projections

**Date:** 2026-07-12
**Status:** Accepted (roadmap - canonical Phase 9 knowledge format and projection boundary)
**Amends:** [ADR 0010](0010-okf-conversation-schema.md) and [ADR 0017](0017-knowledge-base-compiler.md)
**Builds on:** [ADR 0011](0011-vector-rag-retrieval.md), [ADR 0016](0016-auth-identity-bridge.md), and [ADR 0018](0018-three-repo-topology.md)
**Pins:** Google Open Knowledge Format v0.1 draft at upstream commit [`d44368c15e38e7c92481c5992e4f9b5b421a801d`](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/d44368c15e38e7c92481c5992e4f9b5b421a801d/okf/SPEC.md)

## Context

Yap's earlier knowledge ADRs independently converged on Markdown, YAML frontmatter, Git history, cross-links, and disposable indexes. They did not explicitly adopt Google's Open Knowledge Format (OKF), and ADR 0010 deliberately left the canonical Phase 9 schema pending.

Google OKF v0.1 now defines a vendor-neutral interoperability surface:

- a knowledge bundle is a directory tree of UTF-8 Markdown files;
- one concept is one file, and the bundle-relative path without `.md` is its concept ID;
- every concept has YAML frontmatter with a required non-empty `type`;
- `index.md` and `log.md` are reserved for progressive disclosure and change history;
- normal Markdown links assert relationships between concepts;
- unknown types and extension fields remain valid;
- storage, serving, query infrastructure, authorization, and domain taxonomy are intentionally outside the base format.

This file-first format fits Yap's human-readable, Git-compatible, agent-consumable knowledge goal. It does not by itself solve enterprise governance. Yap must maintain meetings, people, projects, decisions, dependencies, follow-up work, policies, and derived agent artifacts while preventing restricted node names, links, backlinks, search hits, counts, and inferred relationships from leaking across permission boundaries.

Vector retrieval alone finds semantically similar chunks but does not make multi-hop relationships authoritative. A graph database can traverse relationships, but it must not become the source of knowledge or the authority that grants access. Git, Postgres, Redis, embeddings, and any optional graph service also cannot share one transaction, so a partially rebuilt projection must never become visible.

Yap needs one explicit decision for the canonical format, enterprise extensions, relationship authority, permission algebra, replaceable relationship/vector projections, and generation-promotion protocol.

## Decision

### 1. Adopt pinned Google OKF v0.1 as the canonical curated format

The team profile's curated Lane 2 knowledge in `yap-knowledge` must conform to the pinned Google OKF v0.1 revision above. The pin prevents a moving draft from silently changing Yap's contract. Adopting a later upstream revision requires conformance evidence and an ADR amendment.

The base rules are normative:

- Concept documents are Markdown with YAML frontmatter and a non-empty `type`.
- The path is the portable OKF concept ID.
- `index.md` and `log.md` follow the reserved-file semantics.
- The bundle root contains `index.md` with `okf_version: "0.1"`; directory indexes provide progressive disclosure without enumerating inaccessible content in served views.
- Absolute bundle-relative and normal relative Markdown links are supported.
- Unknown types and frontmatter fields are preserved across compiler round trips.
- Broken links remain representable and diagnostically visible; they do not make the entire bundle invalid.

Lane 1 raw meetings, live captures, and imports remain content-addressed append records. They become canonical OKF only after normalization, validation, provenance attachment, and the applicable curation policy. This avoids Git commit storms and prevents an unreviewed model extraction from becoming enterprise truth merely because it was written quickly.

### 2. Define a backward-compatible Yap Enterprise OKF profile

Google OKF is intentionally minimally opinionated. Yap adds a versioned profile without making generic consumers depend on it.

Every canonical Yap concept requires:

| Field | Rule |
|-------|------|
| `type` | Google OKF required concept type |
| `title` | Human-readable title |
| `resource` | Stable `yap://` URI for long-lived graph identity |
| `timestamp` | ISO 8601 time of the last meaningful canonical change |
| `yap_schema` | Yap profile version |
| `provenance` | Source concept/version or authorized creator reference |

`description` and `tags` remain strongly recommended. Domain types may add validated fields. The initial canonical vocabulary includes `Person`, `Team`, `Project`, `Meeting`, `Decision`, `ActionItem`, `Artifact`, `System`, `Policy`, `Runbook`, `Metric`, and `Reference`; unknown types remain consumable as generic concepts.

Google's path-based concept ID remains the portable address. The stable `resource` URI is the graph identity used across file moves and renames. A move creates path history or a redirect record rather than creating a different person, project, or decision. Tenant-scoped resources use opaque identifiers, not mutable display names.

### 3. Represent typed relationships without losing OKF interoperability

Normal Markdown links remain mandatory for human and generic-agent navigation. Yap may additionally declare machine-readable relationships in extension frontmatter:

```yaml
---
type: Decision
title: Adopt HTTP/3 at the secure edge
resource: yap://tenant/00000000-0000-0000-0000-000000000000/decision/019f-example
timestamp: 2026-07-12T15:30:00Z
yap_schema: 1
provenance:
  source: /meetings/2026-07-12-voiceos-architecture.md
  source_revision: 4
relationships:
  - type: affects
    target: /projects/voiceos.md
    authority: human_confirmed
  - type: supersedes
    target: /decisions/legacy-transport.md
    authority: asserted
---
```

The Markdown body also links to the target concepts. Generic OKF consumers see a navigable graph even when they do not understand Yap's typed extension.

Relationship authority is explicit:

| Authority | Meaning | Canonical automatically? |
|-----------|---------|--------------------------|
| `asserted` | Written in an accepted authoritative source | Yes, after validation |
| `human_confirmed` | Explicitly accepted through the review workflow | Yes |
| `derived` | Deterministically computed from canonical facts | Yes, as a rebuildable projection |
| `agent_proposed` | Model-inferred relationship with evidence/confidence | No; proposal or agent artifact only |

Every relationship projection records the source path, source commit/content hash, exact source span when available, relationship type, authority, compiler version, permission hash, and build generation. A model prediction cannot authorize its own promotion to canonical knowledge.

### 4. Keep sources and projections separate

The canonical team-profile boundaries are:

| Layer | Authority and responsibility |
|-------|------------------------------|
| Lane 1 append store | Immutable raw captures and machine-write revisions; not automatically curated OKF |
| `yap-knowledge` Git/OKF | Canonical curated concepts, Markdown links, relationship assertions, schemas, permission policy, history, review, and blame |
| Postgres | Authoritative compiled permission ledger, document/resource registry, typed relationship baseline, lineage, audit, build generations, and active-build pointer |
| pgvector | Required initial semantic retrieval baseline inside Postgres; exact or benchmarked HNSW/IVFFlat indexes remain disposable |
| Redis | Optional short-lived cache of compiled allowed concept/resource IDs by principal and build |
| Neo4j | Optional disposable graph/vector challenger for GraphRAG and deep multi-hop traversal; absent from the required baseline |
| Object storage | Raw blobs, immutable snapshots, exports, and backups under retention policy |

The required Phase 9 baseline is Google OKF/Git plus Postgres typed relationship tables and pgvector. That baseline supports directory navigation, metadata filtering, semantic retrieval, permission compilation, and bounded multi-hop traversal without another service.

Neo4j is the preferred graph challenger, not a preselected dependency. It may replace the relational relationship projection, the vector adapter, or both only after enterprise-shaped fixtures demonstrate a material multi-hop quality or latency benefit that justifies its licensing, resource, backup, rebuild, and operational cost. Failure to clear that gate leaves the Postgres/pgvector baseline as the production design.

All projections are rebuildable. Deleting Redis, pgvector indexes, an optional Neo4j projection, or another retrieval adapter must not delete canonical knowledge or permission policy.

### 5. Let query projections construct views, never grant access

Authentication and authorization follow ADR 0016. The validated token-derived `(tenant_id, subject_id)` identifies the principal. The server resolves that principal against the active Postgres permission build.

For a user `U`, agent `A`, task purpose `T`, and active generation `G`, the visible view is the intersection:

```text
Allowed(U, A, T, G)
  = UserPermission(U, G)
  ∩ AgentCapability(A)
  ∩ PurposeScope(T)
  ∩ ClassificationAndRetentionPolicy(G)
```

The baseline relational projector and any promoted graph service may construct the permitted virtual tree or graph only from this compiled allowlist. They do not infer, expand, or override the allowlist. Agents never receive unrestricted SQL, Cypher, retrieval-index, or raw `yap-knowledge` repository access. Yap-owned parameterized query services always inject tenant, active generation, and permission constraints.

One tenant-scoped total relationship projection is maintained per build; Yap does not create one physical database or graph per user. Group-level or principal-level views may be cached by immutable permission/build hash.

### 6. Prevent graph-shaped information leaks

Permission applies independently to concepts, relationships, chunks, backlinks, and derived artifacts.

For principal `P`:

```text
VisibleNodes(P) = nodes explicitly allowed for P

VisibleEdges(P) = edges allowed for P
                  whose source and target nodes are both visible
```

The following rules are mandatory:

- A relationship is hidden when either endpoint or its supporting evidence is not visible.
- Hidden node titles, paths, resource IDs, types, counts, degrees, backlinks, snippets, and vector scores are not exposed.
- Search applies tenant/build/access filters before retrieval when supported and always rechecks candidates against the Postgres ledger before return.
- A synthetic directory ancestor may be shown only to preserve navigation to an allowed descendant; it exposes no hidden sibling names, counts, descriptions, or metadata.
- Cross-tenant traversal fails closed even if identifiers collide.
- Agent artifacts inherit the strictest effective permissions of every source: audience intersection, denial union, and most-restrictive classification.
- Revocation invalidates query caches and makes stale projection generations ineligible immediately through the active permission ledger.

Any graph algorithm or vector procedure operates only on an authorized induced view or returns candidates that undergo the same final authorization check.

### 7. Promote compiled generations atomically

Git, Postgres, Redis, embedding services, and any optional graph service cannot commit one distributed transaction. The compiler therefore publishes immutable build generations:

1. Receive a Lane 1 normalization event or Lane 2 Git commit.
2. Parse and validate OKF, Yap profile fields, links, relationships, provenance, and permission policy.
3. Produce a deterministic intermediate representation of concepts, chunks, relationships, permissions, and content hashes.
4. Compile the next generation into Postgres relationship tables, pgvector, any promoted graph/vector adapter, and the optional Redis cache under a non-active build ID.
5. Validate counts, hashes, link resolution, tenant boundaries, permission invariants, and projection completeness.
6. Atomically update the Postgres `active_build` pointer only after every required projection passes.
7. Pin each query to one active build and permission hash for its lifetime.
8. Retain the previous generation for bounded rollback, then garbage-collect it under policy.

A failed compile leaves the prior generation active. Users never observe new vectors with old permissions, new graph edges with missing nodes, or deleted documents that remain searchable.

Incremental builds may update only changed concepts and affected relationship/permission closures, but their result must be equivalent to a full rebuild from the same source revisions.

### 8. Keep inferred updates reviewable and current

Meeting processing may create proposed people, projects, decisions, action items, and relationships from retained source evidence. These enter Lane 1 or an immutable agent-artifact/proposal path with citations. They do not silently overwrite canonical Lane 2 concepts.

Accepted changes use the same Git review or deployment-approved policy path as human edits. Supersession, deprecation, completion, reassignment, and cancellation are represented as revisioned facts or typed relationships, not destructive history rewrites. Every answer can cite an exact OKF path, source revision/commit, and span.

## Required verification gates

Phase 9 cannot claim this ADR complete without automated evidence for:

| Gate | Required evidence |
|------|-------------------|
| Google OKF conformance | Required `type`, reserved files, supported links, permissive unknown fields/types, and pinned-version fixtures |
| Yap profile | Stable `resource` identity, schema validation, unknown-field preservation, file move/rename, redirect, and duplicate-resource rejection |
| Relationship authority | Asserted/confirmed/derived/proposed transitions, provenance, source spans, supersession, broken targets, and deterministic edge rebuild |
| Determinism | Identical source revisions produce identical intermediate records, graph identities, permission hashes, and chunk identities |
| Permission isolation | Hidden node/edge/title/path/count/backlink/vector-score tests; strict artifact inheritance; cross-tenant and stale-build rejection |
| Atomic generation | Crash/failure injection at every compile stage leaves the previous build active; successful promotion never mixes generations |
| Revocation | Permission withdrawal blocks retrieval and traversal immediately through the active ledger and invalidates caches |
| Multi-hop quality | Licensed fixtures for meeting -> decision -> project -> action and dependency/deprecation questions with exact citations |
| Retrieval benchmark | Postgres/pgvector baseline measured first; Neo4j challenger compared for multi-hop accuracy, filtered recall, hybrid rank quality, p50/p95 latency, CPU, RSS, storage, rebuild time, licensing, and operations |
| Operational recovery | Full projection deletion and deterministic rebuild, rollback, backup/restore, schema-version migration, and orphan cleanup |

## Consequences

### Positive

- Yap uses a real vendor-neutral format instead of a similarly named private convention.
- Humans and generic agents can browse, diff, review, and move knowledge without a Yap SDK.
- Typed, provenance-backed graph relationships support enterprise multi-hop questions without sacrificing ordinary Markdown links.
- Permission-safe virtual trees let users and agents traverse one total corpus without exposing the total corpus.
- The low-complexity Postgres/pgvector baseline remains useful even if no graph service is promoted.
- Neo4j remains available for evidence-backed GraphRAG promotion without coupling canonical knowledge to it.
- Stable resource identity survives file moves while preserving Google OKF path interoperability.
- Generation promotion makes multi-store updates observable as one coherent build.

### Negative

- Yap must maintain a profile validator, deterministic compiler, relationship schema, projection adapters, and migration tests.
- A promoted Neo4j deployment would add an operational and supply-chain surface beyond Postgres/Redis/Git/object storage.
- Permission-safe graph and vector retrieval requires adversarial leak testing, not only functional search tests.
- File moves require path history or redirects because Google OKF identifies concepts by path.
- Curated knowledge may lag raw meeting extraction while proposals await approval.

### Neutral

- Google OKF remains a format, not a serving platform or authorization system.
- The solo/local profile may continue to use files plus SQLite/`sqlite-vec` while consuming the same OKF base format.
- Postgres remains the permission/audit authority and required retrieval baseline even if Neo4j wins the graph benchmark.
- A future OKF version or Neo4j replacement changes a compiler adapter, not the canonical knowledge corpus.

## Alternatives considered

### Continue the homegrown Yap OKF schema

Rejected. It duplicates a now-published interoperability standard, lacks a pinned external conformance target, and makes exchange with generic OKF producers and consumers harder.

### Stop permanently at files plus vector search

Accepted as the required team baseline when combined with Postgres typed relationship tables. Rejected only as a permanent architectural ceiling: if representative impact, supersession, provenance, dependency, ownership, and follow-up queries cannot be answered reliably with the baseline, a graph challenger may earn promotion.

### Store canonical knowledge directly in Neo4j

Rejected. It loses file/Git portability, human review, line-level citations, producer independence, and deterministic rebuild from open artifacts.

### Let Neo4j evaluate authorization dynamically from the graph

Rejected as the permission authority. Graph traversal may construct a view from a compiled allowlist, but authorization remains token-derived and Postgres-ledger-backed so it is auditable, versioned, deterministic, and fail-closed.

### Materialize one graph/tree per user

Rejected. Per-user copies multiply storage, rebuild cost, cache invalidation, revocation latency, and drift risk. One tenant/build projection plus compiled permission views is simpler and safer.

### Make every agent extraction canonical immediately

Rejected. A model cannot validate its own knowledge mutation. Proposed relationships and summaries require provenance plus an explicit acceptance policy.

## Implementation sequence

1. Add pinned Google OKF v0.1 fixtures, conformance validator, and unknown-field round-trip tests.
2. Specify the Yap Enterprise OKF profile and stable `yap://` resource identifiers.
3. Build the deterministic concept/chunk/relationship/permission intermediate representation.
4. Implement Postgres document/resource, permission, lineage, audit, build, and active-generation schemas.
5. Implement Postgres typed relationship tables plus the pgvector semantic-retrieval baseline.
6. Add permission-safe tree, vector, and bounded multi-hop query services; expose no raw SQL or retrieval surface to agents.
7. Implement staged generation validation, atomic promotion, rollback, revocation, and garbage collection.
8. Benchmark Neo4j against the complete baseline with licensed enterprise-shaped fixtures and predefined promotion criteria.
9. Implement the Neo4j graph/vector adapter only if the challenger clears its quality, isolation, operations, and cost gates.
10. Add Lane 1 proposal extraction and reviewed promotion into canonical Lane 2 OKF.
11. Publish a compatibility matrix and migration process before adopting a later Google OKF revision.

## References

- Google Cloud, [Open Knowledge Format v0.1 specification at the pinned revision](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/d44368c15e38e7c92481c5992e4f9b5b421a801d/okf/SPEC.md)
- Google Cloud, [Open Knowledge Format repository at the pinned revision](https://github.com/GoogleCloudPlatform/knowledge-catalog/tree/d44368c15e38e7c92481c5992e4f9b5b421a801d/okf)
- Google Cloud, [Introducing the Open Knowledge Format](https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing)
- Neo4j, [Vector indexes](https://neo4j.com/docs/cypher-manual/current/indexes/semantic-indexes/vector-indexes/)
- Neo4j, [Graph Data Science similarity algorithms](https://neo4j.com/docs/graph-data-science/current/algorithms/similarity/)
- pgvector, [Open-source vector similarity search for Postgres](https://github.com/pgvector/pgvector)
