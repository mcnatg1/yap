# ADR 0020: Meeting capture and diarization authority

**Date:** 2026-07-10
**Status:** Accepted (roadmap - canonical Phase 8)
**Supersedes:** [ADR 0015](0015-two-pass-diarization-speaker-identity.md)
**Supersedes diarization details in:** [ADR 0004](0004-background-diarization-okf-agents.md)
**Amends:** [ADR 0006](0006-silero-agents-state-machine.md), [ADR 0007](0007-forced-alignment-engine.md), [ADR 0009](0009-knowledge-worker-protocol.md), [ADR 0014](0014-server-tier-compute-topology.md), [ADR 0016](0016-auth-identity-bridge.md), [ADR 0018](0018-three-repo-topology.md), and the [Local Audio Preprocessing Stack](../specs/local-audio-preprocessing-stack.md)

## Context

The earlier diarization decisions no longer describe one coherent product:

- ADR 0004 specifies a local WeSpeaker and spectral-clustering vault.
- ADR 0015 moves live and post-meeting diarization to a server-side ECAPA/VBx service.
- The local preprocessing spec says all diarization is server-owned.
- The current desktop captures one microphone stream and couples capture, recording, and local ASR more tightly than a future meeting pipeline can tolerate.

Yap needs a thin client that remains useful without a server while preserving a clean path to server-authoritative meeting processing. Live dictation must not depend on speaker processing. Meeting capture may eventually contain two physical sources, microphone and system loopback, while a diarization model may expose several temporary speaker slots inside either source. These are different concepts and must not share one `source` field.

Speaker identity also introduces a stronger privacy boundary than anonymous diarization. A voice embedding used to identify a person is biometric personal data. Contact names, aliases, and meeting-roster metadata do not require a voiceprint and must not silently cause passive enrollment.

## Decision

### 1. Separate dictation from meeting processing

Yap has two product modes with different critical paths:

| Mode | Required capture | Speaker processing | Result authority |
|------|------------------|--------------------|------------------|
| **Dictation** | Microphone | None | Local or server ASR result according to route policy |
| **Meeting** | Microphone; optional system loopback later | Local anonymous evidence when available; server reconciliation when connected | Server result is authoritative when present |

Diarization never blocks microphone capture, local dictation ASR, text injection, or recording persistence.

Product mode is independent from session origin. `SessionOrigin` is `LiveCapture` or `ImportedFile`; an imported file may contain unknown or mixed physical provenance and must not be mislabeled as microphone or system loopback.

### 2. Use four distinct identifiers

The data model must keep these identifiers separate:

| Identifier | Scope | Meaning |
|------------|-------|---------|
| `track_id` | Session | One captured physical source, such as microphone or system loopback |
| `local_slot_id` | Analysis window | One temporary speaker output produced by an overlap-aware model |
| `session_speaker_id` | Session | One anonymous person cluster, displayed as `Speaker N` |
| `identity_id` | Tenant/server | One purpose-authorized durable identity, keyed independently from its display name |

The number of capture tracks is normally one and eventually at most two. The number of local speaker slots is model-specific. The number of session speakers is dynamic.

### 3. Preserve source tracks and one session timeline

Capture adapters emit timestamped frames onto a common monotonic session timeline. Microphone and system loopback remain separate through persistence and diarization. Mixing is a derived playback or ASR artifact, never the only retained representation.

Every track has its own descriptor, sample-rate history, sequence, and gap events. Device or format changes emit a revisioned track-configuration event. Source-clock position is mapped to the common session clock through revisioned clock-mapping events so drift correction is reproducible. Callback drops and unavailable tracks are explicit. A missing interval must not be hidden by concatenating the remaining samples.

Diarization emits end-exclusive speaker intervals `[start_ms, end_ms)` on that session timeline. Concurrent speakers are represented by overlapping intervals rather than forced into one label. Forced alignment adds word-level `[start_ms, end_ms)` intervals to raw transcript words, then maps each word to the speaker turn with majority overlap. Segment and word timings are result-revision data; later server reconciliation may improve them without changing capture history.

The real-time callback does not allocate, block, or depend on capacity in the ordinary event queue to report loss. Each track owns an atomic dropped-interval accumulator containing the first dropped source position, total dropped frames, and a monotonic loss generation. The coordinator drains it with an atomic swap/compare-exchange before the next accepted frame and again during finalization; callback updates that race a drain remain in the next generation. The drained snapshot becomes deterministic `Gap` events. Consecutive losses may coalesce only when their source positions are contiguous and their cause is identical.

Prepared frames fan out to independent bounded sinks:

```text
capture adapters
  -> timeline and deterministic preprocessing
       -> recording sink
       -> live ASR sink
       -> provisional speaker-evidence sink
       -> future server-transport sink
```

Failure or backpressure in one sink cannot silently discard data from another sink.

### 4. Make local labels anonymous and revisioned

The canonical user-visible speaker states are:

| State | Meaning | May update a durable profile? |
|-------|---------|-------------------------------|
| `Unknown` | Insufficient evidence for a trustworthy cluster assignment | No |
| `Speaker N` | Stable anonymous cluster scoped to one session | No |
| Named assertion | Server matched a purpose-authorized profile and supplied identity provenance | Only through an independently authorized update flow |

`Unknown` and `Speaker N` are not failures. A server-final result may still contain unknown speakers.

The local clustering lifecycle is `Unknown -> Candidate -> StableAnonymous`. `Candidate` is an internal state and remains rendered as `Unknown`; it exists so one noisy or short turn cannot make labels appear and disappear. Promotion to `StableAnonymous` requires calibrated quality, score, runner-up margin, and repeated or cumulative evidence. Once published as `Speaker N`, an attribution remains stable within that immutable result revision. Demotion, merge, or split produces a later result revision instead of mutating the visible label in place.

A user-assigned contact or text label is a display annotation on `Speaker N`, not a named biometric assertion. It carries `user` provenance, may override presentation in that transcript, and never creates or updates a voice profile.

Speaker IDs are stable within a result revision. Reprocessing writes a new immutable result revision. Merges create redirects from old session IDs; splits create new IDs. Manual user corrections take precedence over later automatic display changes unless the user explicitly accepts the revision.

### 5. Weight short and uncertain evidence instead of forcing identity

An embedding from less than 1.6 seconds of clean speech is weak evidence. By itself, it cannot establish a stable session speaker, produce a named assertion, seed a durable profile, or update a profile centroid. It may remain unknown, inherit a surrounding high-confidence speaker through temporal smoothing, or contribute with reduced weight to later session clustering.

Several consistent short turns may eventually establish an anonymous session speaker. A strong match requires all of the following:

- enough cumulative clean speech for the selected embedding model;
- a calibrated score above the source-specific acceptance threshold;
- a calibrated margin over the runner-up;
- acceptable noise, overlap, echo, and clipping quality;
- compatible embedding model, normalization, and calibration versions.

Thresholds are versioned calibration artifacts, not universal constants in an ADR.

### 6. Bound local work without fixing the meeting to four people

Session speaker storage is dynamic. The initial product target is 32 anonymous speakers with a safety ceiling of 64. Above the safety ceiling, Yap retains speech as unknown and marks the session for authoritative reprocessing rather than growing memory or assignment state without bound.

The first local implementation reuses the existing `sherpa-onnx` runtime for speaker embeddings and a measurable anonymous clustering baseline. That baseline does not ship merely because it runs: it must pass the accuracy, callback-drop, CPU, memory, and local-ASR latency gates in the source-aware design. SphereVBx-PF is the preferred clustering challenger because it avoids a separately trained PLDA backend. EEND-VC with MS-SphereVBx remains an overlap-quality challenger, not an initial dependency.

Exact multi-stream inference must have a state budget. Candidate pruning or one-to-one per-window assignment replaces exhaustive joint assignment when the budget would be exceeded. A more complex backend is promoted only after it beats the baseline on licensed meeting fixtures and remains within CPU, memory, latency, and licensing budgets.

### 7. Keep server reconciliation authoritative but optional for capture

When connected, the server may re-run segmentation, embeddings, clustering, overlap handling, alignment, and identity matching from the retained source audio. Client VAD and speaker evidence are hints, not authority. Client false negatives must never remove audio from the server input.

When disconnected:

- dictation continues through the local fallback;
- meeting audio and anonymous results remain available locally;
- official named attribution waits for a server or remains anonymous;
- reconnect creates a retryable, idempotent reconciliation job;
- incomplete source audio can produce only a `partial` result.

Logical chunk identity and byte identity are separate. The idempotency key contains schema version, owner namespace when server-bound, session ID, track ID, and sequence range. `content_sha256` identifies the bytes. Replaying the same key and hash is idempotent; replaying the same key with a different hash is a conflict; different keys with equal hashes are valid unless a higher-level deduplication policy says otherwise. The client rejects mixed-session frames, incompatible rates without an explicit conversion record, conflicting replays, and silent gaps. A Rust-owned durable job ledger is required before automatic reconnect drain ships.

### 8. Separate contacts from biometric profiles

Future contact integration may create Yap-local contacts or import names, aliases, organization, email addresses, meeting-roster membership, and an opaque OS contact reference after explicit user permission. A contact record contains no voice embedding. Import permission, refresh, cache expiry, and deletion are independently controllable; revoking OS access stops refresh and removes imported contact metadata, while an explicitly user-authored transcript label may remain under that transcript's retention policy.

A user may manually map `Speaker N` to a contact for one transcript. Yap may use roster and calendar context to suggest likely contacts, but it does not claim a voice match. Persisting the display label does not create a reusable voice profile.

Automatic cross-session voice matching is available only for an explicitly enrolled profile with a documented legal basis and active purpose authorization. Named profile matching remains server-owned for the team profile. A contact may be linked to the returned identity only through an explicit identity mapping; name or email similarity is not identity proof. Voice embeddings are never shared peer-to-peer or embedded in contact exports.

If a server later matches an enrolled person, it may publish proposed named revisions for the current session and other authorized retained sessions. This backfill uses server recomputation and profile provenance; it does not convert a manual contact label into training consent.

Client-side session embeddings and centroids are transient. Yap persists the speaker timeline and user labels, then discards derived embeddings. If later server reconciliation is needed, the server recomputes evidence from retained audio. This avoids retaining guest biometrics merely for convenience.

Recorded audio remains personal data and may be capable of producing new biometric evidence when reprocessed. It follows a separate recording notice, access, retention, and deletion policy. Discarding derived embeddings does not make retained audio anonymous.

An encrypted local reusable voice-profile feature is out of scope. It requires its own ADR, privacy review, consent and withdrawal flow, retention policy, model-version migration, and threat model.

The meeting feature must expose retention and deletion rather than hiding indefinite storage behind a cache label. The engineering defaults are:

| Data class | Default lifecycle | Deletion behavior |
|------------|-------------------|-------------------|
| In-memory embeddings, centroids, and exemplars | Session only; never serialized | Zero/release on finalization, cancellation, or process exit |
| Incomplete recording artifacts | Recoverable for at most 24 hours | Background cleanup after the recovery window; never shown as complete |
| Pending server-upload job and source artifact | Seven days while retryable | Expiry cancels the job and deletes its private source copy unless the user separately retained the recording |
| Completed meeting audio, transcript, anonymous timeline, and contact labels | Thirty days unless a visible user or organization policy selects another lawful period | One deletion operation invalidates queued jobs and removes linked local artifacts and indexes |
| Imported OS-contact cache | At most 24 hours since the last authorized refresh | Permission withdrawal stops refresh and removes the imported cache |
| User-created Yap contact | Until explicit deletion or account deletion, with an annual stale-contact review | Export/delete controls; never contains an embedding |
| Durable enrolled voice profile | Feature disabled until a deployment defines a finite expiry or review period | Withdrawal makes every profile revision non-matchable immediately, then purges active records, caches, replicas, and backups under the documented SLA |

These are product safety defaults, not a claim that one retention period satisfies every jurisdiction. A deployment policy may shorten them or replace them after legal review, but it cannot silently choose perpetual retention. Backup restore must honor deletion tombstones so removed profiles or recordings do not become active again.

### 9. Treat profile adaptation as a separate authorized operation

A model prediction cannot validate its own training update. Server reconciliation may propose profile evidence, but profile adaptation requires an independent authorization rule, such as explicit enrollment material, user confirmation, or an organization-approved verified workflow.

Enrollment, matching, and adaptation are separate purpose grants. A durable grant records tenant and subject, grant ID, purpose, notice version, approved legal-basis reference, optional consent-text version, grant time, optional withdrawal time, and a monotonic revocation epoch. Matching and profile publication recheck current enrollment and matching grants and epochs. Adaptation additionally requires its own active grant and authorization. A withdrawn grant cannot be bypassed by an in-flight job or older model revision.

Every durable profile stores tenant ID, subject ID, model ID and revision, embedding dimension, normalization version, calibration version, purpose-grant IDs and revocation epochs, finite expiry, withdrawal state, and update provenance. Display names are presentation snapshots, not identity keys.

Profile adaptation is an idempotent operation separate from result publication. Its logical key contains tenant, subject, profile model/revision, source session, source result revision, and independent authorization ID. Replaying the same key and evidence hash is a no-op; a conflicting hash fails closed. Consent and revocation epochs are rechecked in the same transaction that commits the new profile revision.

The server derives `(tenant_id, owner_subject_id)` from the validated token, never from a client-supplied owner field. That namespace participates in every durable job, chunk idempotency key, result revision, profile lookup, object-store key, and audit event. A named assertion is valid only when its session, profile, purpose grants, and identity provenance belong to the same tenant.

Consent withdrawal immediately excludes every profile revision from matching and queued updates. Deletion covers active records, caches, replicas, and backups according to the deployment's documented deletion SLA. A non-biometric revocation tombstone remains so restored backups cannot reactivate deleted material.

### 10. Commit recordings and results through an explicit recovery protocol

A recording is complete only when its commit manifest exists. The persistence order is:

1. Append audio and timeline events to private temporary artifacts; flush at bounded intervals.
2. On stop, drain the loss accumulator and close every sink exactly once.
3. Flush the audio, compute its hash, then flush the immutable capture sidecar containing track, clock, gap, and source metadata.
4. Atomically move the finalized audio and capture sidecar into place.
5. Write and atomically publish the small commit manifest last. It names and hashes both artifacts.
6. On restart, artifacts without a valid commit manifest are `partial` recovery candidates, never completed sessions.

On platforms where directory durability differs, the implementation uses the strongest available file flush and atomic-replace primitive and documents the residual power-loss window. A later SQLite ledger may reference a committed artifact or an explicitly partial recovery record, but it does not make a half-written file complete.

Transcript and speaker result revisions are separate immutable artifacts. Each references the capture-manifest hash and is atomically published without rewriting capture history. A rebuildable result index may select the latest verified revision. Upload acknowledgements, result publication, and profile adaptation use owner-scoped idempotency rules.

## Consequences

### Positive

- Dictation latency and reliability remain isolated from meeting enrichment.
- Offline meetings retain useful anonymous labels without pretending they are verified identities.
- The architecture supports one or two physical tracks and many session speakers without conflating either with model output slots.
- Existing `sherpa-onnx` infrastructure can provide the first benchmarkable client path without another runtime.
- Contacts remain useful without creating a passive biometric address book.
- Server reconciliation can improve results without destroying local or manually corrected history.

### Negative

- Track-aware persistence, gap events, result revisions, and a durable reconnect ledger add real contract work before server streaming.
- Anonymous local and named server results require careful UI language and revision handling.
- Reliable overlap handling may require a larger model after the baseline ships.
- Named voice profiles require deployment-specific legal, security, retention, and consent work.

### Neutral

- System loopback capture remains a separate Windows feature and implementation plan.
- The server may change diarization models without changing the client contract.
- A session can complete successfully with unknown speakers.

## Alternatives considered

### Server-only diarization

Rejected as the only path. It is operationally simple but leaves offline meetings without useful anonymous speaker separation and makes the product unnecessarily brittle during disconnects.

### Full local EEND-VC and MS-SphereVBx first

Rejected as the first implementation. It offers stronger overlap modeling but adds model, licensing, CPU, memory, and assignment-state risk before the capture and evidence contracts are reliable.

### Persistent local guest voice cache

Rejected by default. Renaming it a cache does not remove its biometric-identification purpose. Contacts and per-transcript labels provide most of the UX value without passive cross-session voice tracking.

## Implementation sequence

1. Version the track, clock, gap, evidence, attribution, and result-revision contracts.
2. Harden manifest validation, owner-scoped idempotency, and content identity.
3. Decouple capture, recording, ASR, and speaker-evidence sinks and add callback-safe loss reporting.
4. Add crash-safe bounded recording, commit manifests, versioned sidecars, and recovery tests.
5. Add the local anonymous baseline, hidden candidate lifecycle, and calibrated confidence projection.
6. Add the Rust-owned reconnect ledger with the real server connector.
7. Add server reconciliation, purpose grants, deletion tombstones, and authorized identity matching.
8. Specify Windows system loopback separately.
9. Benchmark SphereVBx-PF and overlap-aware challengers before promotion.

## References

- Delcroix et al., [Multi-Stream Extension of Variational Bayesian HMM Clustering](https://arxiv.org/abs/2305.13580)
- Palka et al., [SphereVBx: Spherical Variational Bayes Clustering for Simplified EEND-VC Diarization](https://arxiv.org/abs/2606.24528)
- [BUTSpeechFIT/VBx](https://github.com/BUTSpeechFIT/VBx)
- [DiariZen VBx implementation](https://github.com/BUTSpeechFIT/DiariZen/blob/main/diarizen/clustering/VBx.py)
- European Commission, [GDPR sensitive-data overview](https://commission.europa.eu/law/law-topic/data-protection/rules-business-and-organisations/legal-grounds-processing-data/sensitive-data/what-personal-data-considered-sensitive_en)
- European Commission, [Data protection by design and by default](https://commission.europa.eu/law/law-topic/data-protection/rules-business-and-organisations/obligations/what-does-data-protection-design-and-default-mean_en)
