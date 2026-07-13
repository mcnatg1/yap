# Yap Server Node Setup Runbook

Yap's team profile treats an NVIDIA GB-class server node as a private server tier, not a public service. The desktop stays thin: local Nemotron INT8 is the live/offline fallback. Phase 5 is intended to send authorized long recordings to `yap-server`; Phase 3 currently provides health reachability and durable queued-job ownership only.

The first supported node profile is DGX Spark GB10. A later GB300-class node should keep the same server contract and change only host-specific config: NIC names, CIDRs, GPU/runtime sizing, and deployment capacity.

## Security Shape

Keep three planes separate:

| Plane | Purpose | Exposure |
| --- | --- | --- |
| Management | SSH, recovery, tunnels | Private Ethernet for demos; corporate LAN/VPN later |
| App entrypoint | Future `yap-server` WSS + HTTP | One TLS endpoint, opened only after the router exists |
| Model/runtime internals | Ollama, VNC, dashboard, model pools, databases | Loopback, container network, or SSH tunnel only |

Default rule: a Yap application service is never exposed to the public internet.
Do not infer host isolation from the private cable: the current GB10 also has
Wi-Fi and overlay routes. Corporate access should mean approved LAN/VPN
reachability plus TLS plus auth, not open model ports.

## Current GB10 And Phase 3 Demo Mode

The 2026-07-12 read-only audit found:

- Windows laptop private IP: `192.168.50.63/24`
- Spark private IP: `192.168.50.1/24`
- Spark wired interface: `enP7s7`
- Spark default route: active Wi-Fi, with additional overlay interfaces
- SSH alias: `dgx-spark-eth`
- UFW: active, but effective rules require root to inspect
- External TCP: SSH only; dashboard, Ollama, and Twingate local services are
  loopback-only
- Tailscale: removed after the audit; Twingate/`sdwan0` remains active
- Time: not NTP-synchronized and about 18.6 seconds ahead of the Windows client

Do **not** rerun the baseline setup script on this prepared, multi-purpose host.
Its landing zone and SSH hardening already exist, and a rerun would perform
unnecessary package, firewall, logging, and service operations.

The validated 2026-07-13 smoke used exact immutable release
`099e558a27a747a7a2f24ec4e86f9c13f7604c13` transiently. It was stopped after
validation; no persistent service remains. Any newer SHA requires promotion and
fresh GB10 evidence and must not inherit the `099e558a27a747a7a2f24ec4e86f9c13f7604c13`
result.

Phase 3 uses a loopback-only health process on the GB10:

```bash
YAP_SERVER_HOST=127.0.0.1 \
YAP_SERVER_PORT=18765 \
PYTHONPATH=/srv/yap-server/releases/<git-sha>/server/src \
python3 -m yap_server
```

Forward that loopback port over the private SSH alias from Windows:

```powershell
ssh -o BatchMode=yes `
  -o ExitOnForwardFailure=yes `
  -o ServerAliveInterval=15 `
  -o ServerAliveCountMax=3 `
  -N -T `
  -L 127.0.0.1:18765:127.0.0.1:18765 `
  dgx-spark-eth
```

Point the desktop connector to `http://127.0.0.1:18765`. This opens no GB10
application port, needs no UFW change, and satisfies the connector's
loopback-only HTTP policy. The client must fail closed when the tunnel dies and
must never retry against the Wi-Fi address.

For an explicitly manual Wi-Fi rehearsal, substitute `dgx-spark-lan` only as
the SSH transport alias in the forwarding command. Keep both sides of the
forward and the desktop connector URL at `127.0.0.1:18765`. Do not put the
Wi-Fi/node address in app configuration, and do not add automatic alias or
network failover.

See the [GB10 readiness audit](../research/2026-07-12-gb10-readiness-audit.md)
for the evidence and remaining gates.

## Fresh Dedicated Node Bootstrap

On a genuinely fresh, dedicated demo node, validate the values first without
root or host mutation:

```bash
env \
  YAP_CONFIGURE_PRIVATE_ETHERNET=1 \
  YAP_PRIVATE_IFACE=enP7s7 \
  YAP_PRIVATE_ADDR=192.168.50.1/24 \
  YAP_PRIVATE_SSH_FROM=192.168.50.63 \
  YAP_LAN_SSH_CIDR= \
  YAP_VALIDATE_ONLY=1 \
  bash infra/yap-server-node/setup-server.sh
```

Then run the bootstrap with conservative firewall handling and explicit
desktop/peripheral cleanup:

```bash
sudo env \
  YAP_CONFIGURE_PRIVATE_ETHERNET=1 \
  YAP_PRIVATE_IFACE=enP7s7 \
  YAP_PRIVATE_ADDR=192.168.50.1/24 \
  YAP_PRIVATE_SSH_FROM=192.168.50.63 \
  YAP_LAN_SSH_CIDR= \
  YAP_HARDWARE_PROFILE=dgx-spark-gb10 \
  YAP_FIREWALL_RESET=0 \
  YAP_DISABLE_NOISE_SERVICES=1 \
  bash infra/yap-server-node/setup-server.sh
```

This adds only the direct-management-link SSH rule and does not open an app
port. Because reset is disabled, existing UFW rules remain and must be
inspected separately. Before running it remotely, prove that a second terminal
can connect with `ssh dgx-spark-eth`. Missing `nmcli`, failed profile
activation, or a missing private address now stops setup before UFW changes.

`YAP_DISABLE_NOISE_SERVICES=1` stops desktop/peripheral services. Use it only on
a dedicated node. If incompatible existing UFW rules truly require a reset,
run only from the local console with a tested recovery path and set both
`YAP_FIREWALL_RESET=1` and `YAP_FIREWALL_RESET_CONFIRM=local-console`. The
script validates all app-port inputs before mutation, installs management rules
before re-enabling UFW, and attempts to restore those rules if a later reset
step fails. Treat any reported recovery failure as a console repair condition.

## Product And IT Ownership Boundary

| Owner | Responsibilities |
| --- | --- |
| Product | Configurable HTTPS origin (with loopback HTTP limited to the Phase 3 tunnel), capability and auth-required state gating, no embedded node IP, and fail-closed retry without automatic network failover |
| IT | Internal DNS, ZPA app segment and policy, App Connector placement and redundancy, connector-to-server routing, TLS termination and certificates, firewall source ranges, and Entra policy |

Product configuration cannot substitute for approved network topology, and IT
network reachability does not imply that upload, authentication, or inference
has shipped.

## Corporate LAN/VPN Mode

For corporate use, get these from IT before opening the app endpoint:

- Stable DNS name, for example `yap-server.<corp-domain>`
- DHCP reservation or static IP for the server node, including wireless if the node is intended to live on Wi-Fi
- Client CIDR or VPN CIDR allowed to reach the service
- TLS certificate source, preferably corporate CA or approved internal ACME
- Auth plan from ADR 0016, likely Entra/MSAL bearer tokens

Then run with corporate CIDRs:

All angle-bracket names and CIDRs below are documentation placeholders. Do not
execute firewall/bootstrap changes with placeholders or guessed values; wait
for approved IT topology and change authorization.

```bash
sudo env \
  YAP_CONFIGURE_PRIVATE_ETHERNET=0 \
  YAP_PRIVATE_SSH_FROM= \
  YAP_LAN_SSH_CIDR='<corp-admin-cidr>' \
  YAP_FIREWALL_RESET=0 \
  YAP_DISABLE_NOISE_SERVICES=0 \
  bash infra/yap-server-node/setup-server.sh
```

Only set `YAP_APP_PORT` after `yap-server` exists and has TLS/auth in front of it:

```bash
sudo env \
  YAP_LAN_SSH_CIDR='<corp-admin-cidr>' \
  YAP_APP_PORT=443 \
  YAP_APP_CIDR='<corp-client-or-vpn-cidr>' \
  YAP_FIREWALL_RESET=0 \
  YAP_DISABLE_NOISE_SERVICES=0 \
  bash infra/yap-server-node/setup-server.sh
```

## Zscaler / Wireless Mode

Longer term, prefer Zscaler Private Access or the approved corporate zero-trust path over exposing the server node to a broad wireless subnet.

Target shape:

- Server node joins corporate Wi-Fi or wired LAN with a stable reservation.
- `yap-server` has an internal DNS name approved by IT.
- Zscaler publishes an app segment for that name and port.
- The server node firewall allows the Zscaler connector/client CIDR to the `yap-server` port.
- SSH stays limited to admin CIDRs or Zscaler admin access, not all wireless clients.
- TLS is required at the app entrypoint; auth is enforced above `/health`.

Example once IT gives the Zscaler CIDRs:

The values in this example remain placeholders until IT supplies the actual
DNS, ZPA, routing, certificate, and source-range design. Do not apply the
example as a firewall change by substituting laptop Wi-Fi addresses.

```bash
sudo env \
  YAP_LAN_SSH_CIDR='<admin-cidr-or-empty>' \
  YAP_ZSCALER_SSH_CIDR='<zpa-admin-cidr>' \
  YAP_APP_PORT=443 \
  YAP_APP_CIDR= \
  YAP_ZSCALER_APP_CIDR='<zpa-app-cidr>' \
  bash infra/yap-server-node/setup-server.sh
```

If IT routes Zscaler traffic through connector hosts, use the connector subnet for `YAP_ZSCALER_APP_CIDR`. If clients source NAT directly from a Zscaler client range, use that range instead. Do not guess this value from the laptop's current Wi-Fi IP.

## Baseline Script

`infra/yap-server-node/setup-server.sh` is intentionally small, but it is a
host-mutating bootstrap tool rather than a normal deploy command. It configures:

- `/srv/yap-server/{releases,shared,logs,data,models}`
- SSH key-only access for the configured admin user
- UFW default-deny inbound firewall
- unattended security updates, no automatic reboot
- journald retention
- Docker log rotation when Docker has no existing daemon config
- optional private Ethernet NetworkManager profile
- optional app entrypoint allow rule
- disabled desktop/peripheral noise that does not belong on a server

Copy `infra/yap-server-node/server.env.example` to a local, untracked env file
for repeatable setup. Its defaults do not reset UFW, disable services, open LAN
SSH, or open an application port. Set `YAP_VALIDATE_ONLY=1` for a non-mutating
configuration and host-prerequisite check before every bootstrap run.

For non-fresh or corporate-managed nodes, keep the conservative settings
explicit even though they are the defaults:

```bash
sudo env \
  YAP_FIREWALL_RESET=0 \
  YAP_DISABLE_NOISE_SERVICES=0 \
  bash infra/yap-server-node/setup-server.sh
```

On a fresh dedicated node, opt in to reset/disable behavior only as shown in the
fresh-node section. Never assume that a second run is harmless merely because
the directory and allow-rule operations are repeatable.

## What Not To Do Yet

- Do not open `11000`, `11434`, `5909`, database ports, or model worker ports directly.
- Do not bind the Phase 3 service to `0.0.0.0`, `[::]`, the Wi-Fi address, or an
  overlay address.
- Do not make the server node public-internet reachable.
- Do not reuse cached model weights, Handy model files, or `latest` container
  tags without pinned provenance, licenses, and hashes.
- Do not enable time-sensitive auth, leases, replay windows, or authoritative
  server timestamps until the host clock is synchronized.
- Do not delete Docker images or model caches just because they are large; disk is cheap and redownloading model/runtime layers is slow.
- Do not force headless mode until VNC/DGX Dashboard recovery is no longer useful.

## Verification

After setup:

```bash
ssh dgx-spark-eth 'hostname; uname -r; systemctl --failed --no-pager'
ssh dgx-spark-eth 'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader'
ssh dgx-spark-eth 'timedatectl show -p NTPSynchronized --value'
ssh dgx-spark-eth 'docker run --rm --pull=never --device=nvidia.com/gpu=all nvcr.io/nvidia/cuda:13.0.1-devel-ubuntu24.04 nvidia-smi --query-gpu=name --format=csv,noheader'
ssh dgx-spark-eth 'sudo ufw status verbose'
```

The Docker command creates an ephemeral container; run it only when that runtime
validation is authorized. The firewall command requires an interactive sudo
session and must never receive a password through a script or command line.

The completed Phase 3 proof established private-link SSH, loopback-only health,
a loopback-only Windows forward, command-line production connector `Ready`
while reachable, and `Retrying` in a separate tunnel-refusal invocation. It did
not establish a same-process native UI transition. Cleanup left no Yap process
or local/remote port-18765 listener. No application firewall rule or external bind was
added. Host clock synchronization and root UFW read-back remain separate later
gates; do not infer them from service status alone.
