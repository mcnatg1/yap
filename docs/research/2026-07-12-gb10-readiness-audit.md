# GB10 readiness audit

- **Audited:** 2026-07-12
- **Validation addendum:** 2026-07-13
- **Target:** Dell Pro Max GB10 on the direct Windows-to-GB10 Ethernet link
- **Method:** The original host audit used read-only local and SSH inspection.
  The later Phase 3 gate promoted and ran one immutable, transient Yap release;
  it made no firewall, routing, persistent-service, or external-bind change.

The original sections record the 2026-07-12 pre-implementation host baseline.
They are not current implementation status. On 2026-07-13, the Phase 3
private-link gate was refreshed against exact immutable release
`c3999b7b685dd668165d54b64d1af61e41adad05`. This document is non-normative:
the Phase 3 implementation record and ADRs own the product contract, while this
document owns the observed host and live-smoke evidence.

## Decision

The selected loopback-only SSH-forward design was executed successfully. The
GB10 Ubuntu ARM64/Python 3.12 server, contract, and infrastructure checks passed
50/50. Through the live tunnel, the command-line production connector projected
`Ready`; in a separate tunnel-refusal invocation it projected `Retrying`. This
did not prove a same-process native UI `Ready`-to-`Retrying` transition. Cleanup
left no Yap process or local/remote port-18765 listener, and no external bind or firewall
change was made. The host remains multi-homed and is not an isolated appliance.

| Area | Observed state | Phase 3 decision |
| --- | --- | --- |
| Private management path | Windows `192.168.50.63/24` to GB10 `192.168.50.1/24`; key-only SSH works | Use `dgx-spark-eth`; never place a password in automation |
| Isolation | GB10 also has active Wi-Fi default routes plus overlay interfaces | Treat the host as multi-homed, not air-gapped |
| TCP exposure | SSH is the only externally bound TCP listener observed | Add no application listener outside loopback |
| Host firewall | UFW is active; effective user rules require root to inspect | Tunnel-first needs no new UFW rule; verify rules before any direct bind |
| Server runtime | Python 3.12.3 ran the dependency-free health process successfully | Keep the standard-library health service in an immutable release directory |
| Yap deployment | Exact release `c3999b7b685dd668165d54b64d1af61e41adad05` is retained read-only under `/srv/yap-server/releases/`; no Yap process, port-18765 listener, or Yap service unit remains | Preserve the immutable smoke artifact; a persistent deployment is future work |
| Durable ledger | SQLite 3.45.1 exists on the host | Phase 3's authoritative job ledger remains Rust-owned on the desktop, per plan |
| ASR | No cleared Yap ASR runtime or model is deployed | Advertise all ASR capabilities as false |
| Time | Host NTP is inactive and the GB10 was about 18.6 seconds ahead of the Windows client | Health-only validation may proceed; fix time before auth, leases, replay windows, or server-owned timestamps |

## Verified host baseline

- Ubuntu 24.04.4 LTS on ARM64, kernel `6.17.0-1026-nvidia`.
- NVIDIA GB10, driver `580.159.03`, CUDA 13.0, compute capability 12.1.
- 20 CPU cores, 121 GiB RAM, and about 3.2 TiB free NVMe space.
- No failed systemd units and no pending reboot were observed.
- Docker and the NVIDIA Container Toolkit are installed, but no containers or
  published container ports were present.
- `/srv/yap-server/{releases,shared,logs,data,models}` exists with the intended
  owner and private directory permissions.
- Rust, Cargo, Node, npm, and pnpm are not installed on the GB10. Build desktop
  artifacts on the Windows development machine; do not turn the server into a
  second general-purpose build workstation for Phase 3.

## Network and service truth

The direct link is correctly isolated from default routing:

- Windows interface: `192.168.50.63/24`
- GB10 interface `enP7s7`: `192.168.50.1/24`, no gateway or DNS
- Windows default route remains on Wi-Fi
- `dgx-spark-eth` resolves to the private address and uses a dedicated SSH key

The GB10 itself is multi-homed:

- `wlP9s9` has `192.168.68.61/22` and the IPv4/IPv6 default routes.
- `sdwan0` supplies overlay routes and DNS.
- Twingate and Tailscale were active at audit time.
- `docker0` exists but had no active published workload.

After the read-only audit, the user authorized removing Tailscale from this test
machine. A Tailscale logout was attempted before Tailscale 1.98.8 was stopped,
disabled, and purged; its dedicated APT source, archive key, and
`/var/lib/tailscale` state were removed. Independent verification found no
package, unit, binary,
interface, route, state directory, or UDP 41641 listener. Private Ethernet,
Wi-Fi, and Twingate remained active. The host is therefore still multi-homed
and must retain the tunnel-first Yap boundary.

Observed TCP listeners were:

| Bind | Service | Boundary |
| --- | --- | --- |
| `0.0.0.0:22`, `[::]:22` | OpenSSH | Client-side TCP probes proved reachability on both private Ethernet and Wi-Fi under the current firewall policy |
| `127.0.0.1:11000` | DGX Dashboard | Loopback only |
| `127.0.0.1:11434` | Ollama | Loopback only |
| `127.0.0.1:45239` | Twingate local endpoint | Loopback only |

SSH is public-key-only for the shared `admin` account, with root login,
password authentication, keyboard-interactive authentication, X11 forwarding,
agent forwarding, and SSH tunnels disabled except for local TCP forwarding.
The shared account currently has multiple authorized keys; production access
needs named identity and key ownership review later, but that is outside the
current no-auth Phase 3 boundary.

## Phase 3 tunnel boundary

Port `18765` is the validated development and GB10 smoke boundary.
The validated transient process ran on the GB10 with:

```bash
YAP_SERVER_HOST=127.0.0.1 \
YAP_SERVER_PORT=18765 \
PYTHONPATH=/srv/yap-server/releases/<git-sha>/server/src \
python3 -m yap_server
```

Create a loopback-only forward on Windows:

```powershell
ssh -o BatchMode=yes `
  -o ExitOnForwardFailure=yes `
  -o ServerAliveInterval=15 `
  -o ServerAliveCountMax=3 `
  -N -T `
  -L 127.0.0.1:18765:127.0.0.1:18765 `
  dgx-spark-eth
```

The desktop connector then uses `http://127.0.0.1:18765`. This preserves the
connector's loopback-only HTTP rule and avoids an insecure-private-address
override. The tunnel must fail closed; it must never retry against the GB10's
Wi-Fi address.

This path requires no UFW change because the service has no externally
reachable socket. If a later phase needs a direct private-interface bind, it
must first:

1. Inspect the effective root-owned firewall rules.
2. Bind only to `192.168.50.1`, never `0.0.0.0` or `[::]`.
3. Allow only `192.168.50.63/32` on `enP7s7` and explicitly deny the service
   on Wi-Fi and overlay interfaces.
4. Add the reviewed TLS, organization-origin, and ADR 0016 authentication
   boundary before sending audio or transcript content.

## Runtime and model findings

The health process ran successfully with Python's standard library and did not
need FastAPI, Uvicorn, a container, or a new host package. No Phase 3 health
response may advertise `batchJobs`, `liveStreaming`, or `jobStatus` until a
real implementation exists.

The server contains useful but unapproved donor assets:

- Loopback-only Ollama has text-generation models suitable for later optional
  enrichment, not ASR.
- A dormant Handy installation contains Parakeet and Cohere-named ONNX model
  files, but no co-located license or notice evidence was found.
- Cached ARM64/CUDA 13 container images exist, but cached `latest` tags are not
  a production pin and container GPU execution was not exercised.

Do not copy or deploy those model assets in Yap until license provenance,
hashes, ARM64/CUDA 13 compatibility, accuracy, and performance are proved.
The desktop durable imported-job boundary is implemented. Remote upload, drain,
server ASR, and completed imported-recording processing remain Phase 5 work.

## Phase 3 validation result

Phase 3 completed with the following live-node evidence:

- Exact immutable release:
  `c3999b7b685dd668165d54b64d1af61e41adad05`.
- Deployment archive SHA-256:
  `be7f43d757821c3e74d0ae2809599f5a84b369115d24afce42fe6687b1bf12e1`.
- GB10 Ubuntu ARM64/Python 3.12 server, contract, and infrastructure suite:
  **50/50 passed**.
- Live loopback health through the private-link SSH forward and command-line
  production connector projection: `Ready`.
- Separate dead-tunnel/refused-connection invocation: `Retrying`.
- Compiled/runtime inspection found no node-address or Wi-Fi fallback literal.
- Teardown left no local or remote port-18765 listener and no Yap process.
- No persistent service, upload, WSS runtime, authentication, ASR, model pool,
  external application listener, or firewall change was introduced.

Clock synchronization and root-level firewall inspection are explicit gates
before later time-sensitive auth or direct network exposure. They do not
justify weakening the tunnel-first Phase 3 boundary.
