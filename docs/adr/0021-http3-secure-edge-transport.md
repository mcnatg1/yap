# ADR 0021: HTTP/3 transport evolution at the secure edge

**Date:** 2026-07-12
**Status:** Accepted (roadmap - gated after the Phase 5 remote transport and Phase 7 authentication baselines)
**Amends:** [ADR 0014](0014-server-tier-compute-topology.md)
**Relates to:** [ADR 0001](0001-dual-stt-backends.md), [ADR 0016](0016-auth-identity-bridge.md), and [ADR 0020](0020-meeting-capture-diarization-authority.md)
**Implementation status:** The Phase 5 candidate implements the transport-neutral durable batch contract over a manually SSH-forwarded loopback HTTP/1.1 development boundary. It does not implement authenticated WSS, TLS/QUIC, HTTP/3, UDP exposure, or an enterprise edge; its one-time complete gate is pending.

## Context

Yap has two different network workloads:

- durable control and batch operations such as health, capability discovery, job creation, chunk upload, commit, status, cancellation, and recovery;
- latency-sensitive live audio and transcript events that must remain ordered, replayable, bounded, and reconnectable.

The first private server boundary runs on loopback and is reached through an SSH tunnel over a direct Ethernet link. That boundary deliberately uses the Python standard-library HTTP server so Phase 3 can prove contracts, reachability, cancellation, and durable ownership without adding a framework, TLS termination, UDP exposure, or another operational stack.

HTTP/3 uses QUIC over UDP with TLS 1.3. Independent QUIC streams prevent loss on one request stream from blocking progress on unrelated streams, and QUIC supports connection migration. Those properties may improve Yap's live path and concurrent control traffic on lossy or changing networks. They do not guarantee that HTTP/3 is faster on a reliable private LAN, and UDP can be blocked. The HTTP/3 standard therefore calls for clients to attempt a TCP-based HTTP version when QUIC cannot be established.

Yap needs a long-term transport target without coupling the Phase 3 application server or durable job semantics to one wire protocol before the authenticated baseline exists.

## Decision

### 1. Make HTTP/3 the preferred future client-facing transport

The team/server deployment profile will target HTTP/3 at its authenticated client-facing edge. That edge may be a colocated reverse proxy or a separately managed gateway. It terminates QUIC/TLS and forwards to the Yap application over a private loopback or equivalent local transport.

The deployment must also offer HTTP/2 and/or HTTP/1.1 fallback. Yap will not ship an HTTP/3-only client or server profile.

```text
Yap desktop
  -> HTTPS origin
       -> HTTP/3 over QUIC when negotiated
       -> HTTP/2 or HTTP/1.1 fallback when UDP/QUIC is unavailable
  -> authenticated secure edge
  -> private loopback application service
```

### 2. Keep Phase 3 on the bounded loopback service

Phase 3 continues to bind `127.0.0.1:18765` by default and uses the SSH-tunnel runbook for the GB10 test path. It does not open a UDP port, provision public certificates, advertise `Alt-Svc`, or add an HTTP/3 library.

HTTP method semantics, OpenAPI payloads, error envelopes, idempotency keys, replay rules, content hashes, event sequences, and gap accounting remain transport-independent. Moving the edge from HTTP/1.1 to HTTP/3 must not require a job-contract or recording-format rewrite.

### 3. Establish authenticated WSS before selecting the final live carrier

The first real live-streaming vertical slice uses the already planned authenticated WSS event contract because it provides the simplest baseline for ordering, replay, cancellation, and compatibility.

After that baseline works, benchmark these client-facing candidates through the same secure edge:

1. WSS over HTTP/1.1 or HTTP/2 as the compatibility baseline;
2. WebSockets bootstrapped over HTTP/3 Extended CONNECT as defined by RFC 9220;
3. WebTransport over HTTP/3 when the chosen Rust client, edge, and server implementations are mature enough for a supported product path.

Unreliable QUIC DATAGRAM frames carried through the HTTP Datagram extension are not a license to make audio loss invisible. Any such live-media experiment must still emit exact source gaps and preserve the durable recording/upload path. Reliable Capsule carriage is a distinct fallback and must be measured as such. Final transcript events, job mutations, commits, and acknowledgements remain reliable and idempotent.

### 4. Gate UDP exposure behind the security boundary

No HTTP/3 listener may be enabled on the GB10 or another server until all of the following exist:

- a managed TLS 1.3 certificate and hostname validation path;
- application authentication and authorization for every non-health operation;
- rate limits, request/body bounds, connection limits, and denial-of-service controls;
- structured request, QUIC-handshake, fallback, and transport-error telemetry with content redaction;
- explicit firewall rules limited to the intended interface and deployment profile;
- a tested HTTP/2 or HTTP/1.1 rollback path;
- a threat-model update covering UDP, QUIC connection IDs, migration, replay, and edge-to-app trust.

Loopback remains the application service default even after the edge exists. The edge, not the Python health handler, owns QUIC and TLS.

### 5. Negotiate capability and preserve fallback

A later versioned contract may advertise supported transports and live carriers. Until that schema is accepted, deployment configuration, a direct QUIC attempt, or an HTTPS `Alt-Svc` advertisement selects an HTTP/3 endpoint; the QUIC TLS handshake then negotiates `h3` through ALPN. The desktop treats HTTP/3 as an optimization rather than a required capability.

The connector must record which transport was negotiated, retry through the bounded route policy, and fall back without duplicating a logical job, chunk, commit, or final transcript event. Fallback must never weaken authentication or certificate validation.

### 6. Restrict zero-RTT

Zero-RTT is disabled for authenticated mutations, chunk upload, commit, cancellation, profile operations, and any request whose replay could change durable state. A later security review may permit it for explicitly safe, read-only requests. Idempotency keys do not by themselves make replay of an authenticated mutation acceptable.

### 7. Promote HTTP/3 only with measured evidence

The benchmark must use the same client, payloads, authentication mode, and server hardware across transports. Promotion requires:

| Gate | Required evidence |
|------|-------------------|
| Contract parity | All HTTP/job/live contract and recovery tests pass unchanged through HTTP/3 and fallback paths |
| Reliable-LAN baseline | No material regression in p50/p95 control latency, first-event latency, throughput, or completion rate on the direct Ethernet profile |
| Loss and jitter | Measured p95 improvement in at least one declared target scenario, with no replay, ordering, hash, or gap-accounting regression under 1%, 3%, and 5% induced loss |
| Migration/reconnect | Interface transition and address change do not duplicate jobs/events or lose a committed result; fallback completes within the connector's bounded policy |
| Resource budget | Client and edge CPU, RSS, handles, and power remain within an accepted budget documented before the benchmark |
| Operations | UDP-blocked, certificate-failure, edge-restart, app-restart, and downgrade/fallback drills pass with bounded errors and useful telemetry |
| Security | Threat model and deployment review approve the exact QUIC library, proxy configuration, TLS policy, and exposed interface |

If HTTP/3 does not beat the authenticated fallback baseline under Yap's actual workload, it remains a supported edge experiment rather than a release requirement.

## Consequences

### Positive

- HTTP/3 becomes a concrete roadmap target with acceptance evidence instead of an informal future idea.
- QUIC can improve independent-stream progress and connection continuity on lossy or changing networks.
- The application service remains small, private, and testable while the edge owns protocol complexity.
- Stable contracts and idempotency rules survive transport promotion and fallback.
- WSS and HTTP/3/WebTransport can be compared against the same working live baseline.

### Negative

- The secure edge adds certificate, UDP/firewall, observability, patching, and incident-response work.
- The desktop must test at least one TCP fallback and one QUIC path.
- QUIC traffic can be blocked or treated differently by enterprise networks.
- HTTP/3 and WebTransport libraries add supply-chain and platform-support gates.
- Transport benchmarking requires controlled loss, jitter, migration, and fallback fixtures.

### Neutral

- Phase 3 implementation and its loopback health service do not change.
- Phase 5 owns the first durable remote batch transport. The authenticated WSS
  baseline and Phase 7 authentication boundary remain prerequisites before
  HTTP/3 promotion.
- HTTP/3 does not change which side owns capture, recording, jobs, inference, diarization, or identity.

## Alternatives considered

### Require end-to-end HTTP/3 in Phase 3

Rejected. It would combine contract bring-up with TLS, QUIC, UDP, proxy, certificate, and client-library work before basic reachability and durable ownership are proven.

### Keep HTTP/1.1 permanently

Rejected as the long-term target. It is a useful private application transport and fallback, but it does not let Yap evaluate QUIC stream independence, migration, or HTTP/3 live carriers.

### Expose native QUIC directly from the application server

Deferred. Direct QUIC may eventually be justified, but it couples the application runtime to transport and certificate operations. The secure-edge design provides the benefits with a smaller rollback surface.

### Replace the live contract with unreliable datagrams immediately

Rejected. Live partials may tolerate loss, but source audio gaps, final results, commits, cancellation, and replay ownership require explicit reliable semantics. A datagram experiment must preserve those invariants rather than bypass them.

## Implementation sequence

1. Retain and verify the Phase 3 loopback health service while finishing the connector, cancellation/retry, and Rust SQLite ledger work.
2. Deliver the durable HTTP batch path in Phase 5, then establish authenticated
   WSS together with the Phase 7 security boundary before transport promotion.
3. Select a maintained HTTP/3 edge and Rust client candidate; pin versions and complete license/security review.
4. Add local certificate, QUIC, UDP-blocked fallback, loss/jitter, migration, and restart fixtures without opening a broad LAN listener.
5. Benchmark HTTP/3 WebSockets and a supported WebTransport candidate against the WSS baseline.
6. Add versioned transport capability reporting only after the supported matrix is known.
7. Enable HTTP/3 for a test deployment behind a rollbackable edge; promote it only when every gate above passes.

## References

- IETF, [RFC 9000: QUIC: A UDP-Based Multiplexed and Secure Transport](https://www.rfc-editor.org/rfc/rfc9000.html)
- IETF, [RFC 9114: HTTP/3](https://www.rfc-editor.org/rfc/rfc9114.html)
- IETF, [RFC 9220: Bootstrapping WebSockets with HTTP/3](https://www.rfc-editor.org/rfc/rfc9220.html)
- IETF, [RFC 9297: HTTP Datagrams and the Capsule Protocol](https://www.rfc-editor.org/rfc/rfc9297.html)
