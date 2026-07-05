#!/usr/bin/env bash
set -euo pipefail

: "${YAP_OWNER:=admin}"
: "${YAP_GROUP:=admin}"
: "${YAP_SERVER_ROOT:=/srv/yap-server}"
: "${YAP_HARDWARE_PROFILE:=dgx-spark-gb10}"
: "${YAP_PRIVATE_IFACE:=enP7s7}"
: "${YAP_PRIVATE_ADDR:=192.168.50.1/24}"
: "${YAP_PRIVATE_SSH_FROM=192.168.50.63}"
: "${YAP_CONFIGURE_PRIVATE_ETHERNET:=0}"
: "${YAP_LAN_SSH_CIDR=192.168.68.0/22}"
: "${YAP_OVERLAY_SSH_CIDR=}"
: "${YAP_ZSCALER_APP_CIDR=}"
: "${YAP_ZSCALER_SSH_CIDR=}"
: "${YAP_APP_PORT=}"
: "${YAP_APP_CIDR=}"
: "${YAP_FIREWALL_RESET:=1}"
: "${YAP_DISABLE_NOISE_SERVICES:=1}"
: "${YAP_TUNNEL_ONLY_PORTS:=11000 11434 5909 3389}"

need_root() {
  if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root, for example: sudo env YAP_LAN_SSH_CIDR=... bash setup-server.sh" >&2
    exit 1
  fi
}

valid_user() {
  printf '%s' "$1" | grep -Eq '^[a-z_][a-z0-9_-]*[$]?$'
}

install_basics() {
  apt-get update
  DEBIAN_FRONTEND=noninteractive apt-get install -y unattended-upgrades ufw

  cat >/etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
EOF

  cat >/etc/apt/apt.conf.d/52yap-unattended-server.conf <<'EOF'
Unattended-Upgrade::Remove-Unused-Kernel-Packages "true";
Unattended-Upgrade::Remove-New-Unused-Dependencies "true";
Unattended-Upgrade::Remove-Unused-Dependencies "true";
Unattended-Upgrade::Automatic-Reboot "false";
EOF

  systemctl enable --now unattended-upgrades >/dev/null 2>&1 || true
}

setup_dirs() {
  install -d -m 0750 -o "$YAP_OWNER" -g "$YAP_GROUP" "$YAP_SERVER_ROOT"
  for dir in releases shared logs data models; do
    install -d -m 0750 -o "$YAP_OWNER" -g "$YAP_GROUP" "$YAP_SERVER_ROOT/$dir"
  done

  cat >"$YAP_SERVER_ROOT/README.md" <<'EOF'
# yap-server landing zone

Private NVIDIA GB-class server node workspace for the Yap server tier.

- releases: deploys
- shared: env/config owned outside releases
- logs: app logs
- data: local app state
- models: model/cache payloads

Do not expose WSS/HTTP ports until the workload router service exists.
EOF
}

setup_ssh() {
  if ! valid_user "$YAP_OWNER"; then
    echo "Refusing invalid YAP_OWNER: $YAP_OWNER" >&2
    exit 1
  fi

  install -d -m 0755 /etc/ssh/sshd_config.d
  cat >/etc/ssh/sshd_config.d/98-yap-access.conf <<EOF
AuthenticationMethods publickey
AllowUsers ${YAP_OWNER}
AllowAgentForwarding no
PermitTunnel no
PermitUserEnvironment no
AllowTcpForwarding local
EOF

  cat >/etc/ssh/sshd_config.d/99-server-hardening.conf <<'EOF'
PermitRootLogin no
PasswordAuthentication no
PubkeyAuthentication yes
KbdInteractiveAuthentication no
X11Forwarding no
ClientAliveInterval 60
ClientAliveCountMax 3
EOF

  sshd -t
  systemctl reload ssh
}

setup_private_ethernet() {
  [ "$YAP_CONFIGURE_PRIVATE_ETHERNET" = "1" ] || return 0
  command -v nmcli >/dev/null || {
    echo "nmcli not found; skipping private Ethernet profile" >&2
    return 0
  }

  if nmcli -t -f NAME con show | grep -Fxq laptop-link; then
    nmcli con mod laptop-link connection.interface-name "$YAP_PRIVATE_IFACE" \
      connection.autoconnect yes connection.autoconnect-priority 50 \
      ipv4.method manual ipv4.addresses "$YAP_PRIVATE_ADDR" \
      ipv4.gateway '' ipv4.never-default yes ipv6.method link-local
  else
    nmcli con add type ethernet ifname "$YAP_PRIVATE_IFACE" con-name laptop-link \
      connection.autoconnect yes connection.autoconnect-priority 50 \
      ipv4.method manual ipv4.addresses "$YAP_PRIVATE_ADDR" \
      ipv4.never-default yes ipv6.method link-local
  fi
  nmcli con up laptop-link || true
}

setup_firewall() {
  [ "$YAP_FIREWALL_RESET" = "1" ] && ufw --force reset >/dev/null
  ufw default deny incoming >/dev/null
  ufw default allow outgoing >/dev/null
  ufw default deny routed >/dev/null

  if [ -n "$YAP_PRIVATE_SSH_FROM" ]; then
    ufw allow in on "$YAP_PRIVATE_IFACE" from "$YAP_PRIVATE_SSH_FROM" to any port 22 proto tcp comment 'SSH from private management link' >/dev/null
  fi
  if [ -n "$YAP_LAN_SSH_CIDR" ]; then
    ufw allow from "$YAP_LAN_SSH_CIDR" to any port 22 proto tcp comment 'SSH from LAN/VPN' >/dev/null
  fi
  if [ -n "$YAP_OVERLAY_SSH_CIDR" ]; then
    ufw allow from "$YAP_OVERLAY_SSH_CIDR" to any port 22 proto tcp comment 'SSH from overlay' >/dev/null
  fi
  if [ -n "$YAP_ZSCALER_SSH_CIDR" ]; then
    ufw allow from "$YAP_ZSCALER_SSH_CIDR" to any port 22 proto tcp comment 'SSH from Zscaler/ZPA' >/dev/null
  fi

  for port in $YAP_TUNNEL_ONLY_PORTS; do
    ufw deny in to any port "$port" proto tcp comment 'tunnel-only service' >/dev/null
  done

  if [ -n "$YAP_APP_PORT" ]; then
    app_rules=0
    if [ -n "$YAP_APP_CIDR" ]; then
      ufw allow from "$YAP_APP_CIDR" to any port "$YAP_APP_PORT" proto tcp comment 'Yap server entrypoint' >/dev/null
      app_rules=$((app_rules + 1))
    fi
    if [ -n "$YAP_ZSCALER_APP_CIDR" ]; then
      ufw allow from "$YAP_ZSCALER_APP_CIDR" to any port "$YAP_APP_PORT" proto tcp comment 'Yap server via Zscaler/ZPA' >/dev/null
      app_rules=$((app_rules + 1))
    fi
    if [ "$app_rules" -eq 0 ]; then
      echo "YAP_APP_PORT is set, but no YAP_APP_CIDR or YAP_ZSCALER_APP_CIDR is set" >&2
      exit 1
    fi
  fi

  ufw --force enable >/dev/null
}

setup_logs() {
  install -d -m 0755 /etc/systemd/journald.conf.d
  cat >/etc/systemd/journald.conf.d/99-yap-server-retention.conf <<'EOF'
[Journal]
SystemMaxUse=512M
RuntimeMaxUse=256M
MaxRetentionSec=14day
EOF
  systemctl restart systemd-journald
  journalctl --vacuum-size=512M >/dev/null
}

setup_docker_logs() {
  if [ -f /etc/docker/daemon.json ]; then
    echo "Leaving existing /etc/docker/daemon.json in place"
    return 0
  fi

  install -d -m 0755 /etc/docker
  cat >/etc/docker/daemon.json <<'EOF'
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "100m",
    "max-file": "3"
  }
}
EOF
  systemctl restart docker || true
}

disable_noise() {
  [ "$YAP_DISABLE_NOISE_SERVICES" = "1" ] || return 0
  systemctl disable --now \
    xrdp.service xrdp-sesman.service \
    cups.service cups-browsed.service \
    bluetooth.service ModemManager.service \
    avahi-daemon.service avahi-daemon.socket \
    openvpn.service lldpd.service \
    2>/dev/null || true
  snap stop --disable cups >/dev/null 2>&1 || true
  systemctl reset-failed >/dev/null 2>&1 || true
}

report() {
  echo "== yap server setup =="
  echo "hardware profile: $YAP_HARDWARE_PROFILE"
  hostname || true
  uname -r || true
  echo
  systemctl is-active ssh ufw unattended-upgrades 2>/dev/null || true
  echo
  ufw status verbose || true
  echo
  find "$YAP_SERVER_ROOT" -maxdepth 1 -mindepth 0 -printf '%M %u %g %p\n' | sort
}

need_root
install_basics
setup_dirs
setup_ssh
setup_private_ethernet
setup_firewall
setup_logs
setup_docker_logs
disable_noise
report
