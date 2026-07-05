# Yap Server Node Setup Runbook

Yap's team profile treats an NVIDIA GB-class server node as a private server tier, not a public service. The desktop stays thin: local Moonshine tiny is the live/offline fallback, and official large recordings go to `yap-server` when it is reachable.

The first supported node profile is DGX Spark GB10. A later GB300-class node should keep the same server contract and change only host-specific config: NIC names, CIDRs, GPU/runtime sizing, and deployment capacity.

## Security Shape

Keep three planes separate:

| Plane | Purpose | Exposure |
| --- | --- | --- |
| Management | SSH, recovery, tunnels | Private Ethernet for demos; corporate LAN/VPN later |
| App entrypoint | Future `yap-server` WSS + HTTP | One TLS endpoint, opened only after the router exists |
| Model/runtime internals | Ollama, VNC, dashboard, model pools, databases | Loopback, container network, or SSH tunnel only |

Default rule: the server node is never exposed to the public internet. Corporate access should mean LAN/VPN reachability plus TLS plus auth, not open model ports.

## Demo Mode

For demos and early development on the current DGX Spark GB10:

- Windows laptop private IP: `192.168.50.63/24`
- Spark private IP: `192.168.50.1/24`
- Spark wired interface: `enP7s7`
- Spark internet: Wi-Fi/router or another upstream route
- SSH alias: `dgx-spark-eth`

Run the setup from this repo:

```bash
sudo env \
  YAP_CONFIGURE_PRIVATE_ETHERNET=1 \
  YAP_PRIVATE_IFACE=enP7s7 \
  YAP_PRIVATE_ADDR=192.168.50.1/24 \
  YAP_PRIVATE_SSH_FROM=192.168.50.63 \
  YAP_LAN_SSH_CIDR=192.168.68.0/22 \
  YAP_HARDWARE_PROFILE=dgx-spark-gb10 \
  bash infra/yap-server-node/setup-server.sh
```

This keeps SSH open for the demo link and current LAN, but it does not open an app port.

## Corporate LAN/VPN Mode

For corporate use, get these from IT before opening the app endpoint:

- Stable DNS name, for example `yap-server.<corp-domain>`
- DHCP reservation or static IP for the server node, including wireless if the node is intended to live on Wi-Fi
- Client CIDR or VPN CIDR allowed to reach the service
- TLS certificate source, preferably corporate CA or approved internal ACME
- Auth plan from ADR 0016, likely Entra/MSAL bearer tokens

Then run with corporate CIDRs:

```bash
sudo env \
  YAP_CONFIGURE_PRIVATE_ETHERNET=0 \
  YAP_PRIVATE_SSH_FROM= \
  YAP_LAN_SSH_CIDR='<corp-admin-cidr>' \
  bash infra/yap-server-node/setup-server.sh
```

Only set `YAP_APP_PORT` after `yap-server` exists and has TLS/auth in front of it:

```bash
sudo env \
  YAP_LAN_SSH_CIDR='<corp-admin-cidr>' \
  YAP_APP_PORT=443 \
  YAP_APP_CIDR='<corp-client-or-vpn-cidr>' \
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

`infra/yap-server-node/setup-server.sh` is intentionally small and idempotent. It configures:

- `/srv/yap-server/{releases,shared,logs,data,models}`
- SSH key-only access for the configured admin user
- UFW default-deny inbound firewall
- unattended security updates, no automatic reboot
- journald retention
- Docker log rotation when Docker has no existing daemon config
- optional private Ethernet NetworkManager profile
- optional app entrypoint allow rule
- disabled desktop/peripheral noise that does not belong on a server

Copy `infra/yap-server-node/server.env.example` to a local env file for repeatable setup.

For non-fresh or corporate-managed nodes, avoid destructive baseline changes unless IT has approved them:

```bash
sudo env \
  YAP_FIREWALL_RESET=0 \
  YAP_DISABLE_NOISE_SERVICES=0 \
  bash infra/yap-server-node/setup-server.sh
```

Use the defaults for a fresh demo/server node where resetting UFW and disabling desktop/peripheral services is intended.

## What Not To Do Yet

- Do not open `11000`, `11434`, `5909`, database ports, or model worker ports directly.
- Do not make the server node public-internet reachable.
- Do not delete Docker images or model caches just because they are large; disk is cheap and redownloading model/runtime layers is slow.
- Do not force headless mode until VNC/DGX Dashboard recovery is no longer useful.

## Verification

After setup:

```bash
ssh dgx-spark-eth 'hostname; uname -r; systemctl --failed --no-pager'
ssh dgx-spark-eth 'nvidia-smi --query-gpu=name,driver_version --format=csv,noheader'
ssh dgx-spark-eth 'docker run --rm --pull=never --device=nvidia.com/gpu=all nvcr.io/nvidia/cuda:13.0.1-devel-ubuntu24.04 nvidia-smi --query-gpu=name --format=csv,noheader'
ssh dgx-spark-eth 'sudo ufw status verbose'
```

Expected state: SSH works, apt is clean, no reboot pending, UFW is active, only SSH is externally reachable, and GPU works from host and Docker.
