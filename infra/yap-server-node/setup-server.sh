#!/usr/bin/env bash
set -euo pipefail

: "${YAP_OWNER:=admin}"
: "${YAP_GROUP:=admin}"
: "${YAP_SERVER_ROOT:=/srv/yap-server}"
: "${YAP_HARDWARE_PROFILE:=dgx-spark-gb10}"
: "${YAP_PRIVATE_IFACE:=enP7s7}"
: "${YAP_PRIVATE_ADDR:=192.168.50.1/24}"
: "${YAP_PRIVATE_SSH_FROM=192.168.50.63}"
: "${YAP_SSH_POLICY_TEST_ADDR:=192.168.50.63}"
: "${YAP_CONFIGURE_PRIVATE_ETHERNET:=0}"
: "${YAP_LAN_SSH_CIDR=}"
: "${YAP_OVERLAY_SSH_CIDR=}"
: "${YAP_ZSCALER_APP_CIDR=}"
: "${YAP_ZSCALER_SSH_CIDR=}"
: "${YAP_APP_PORT=}"
: "${YAP_APP_CIDR=}"
: "${YAP_FIREWALL_RESET:=0}"
: "${YAP_FIREWALL_RESET_CONFIRM=}"
: "${YAP_DISABLE_NOISE_SERVICES:=0}"
: "${YAP_TUNNEL_ONLY_PORTS:=3389 5909 11000 11434 18765}"
: "${YAP_VALIDATE_ONLY:=0}"

FIREWALL_RESET_IN_PROGRESS=0
YAP_VALIDATION_PYTHON=

die() {
  echo "$1" >&2
  exit 1
}

on_exit() {
  status=$?
  trap - EXIT
  if [ "$status" -ne 0 ] && [ "$FIREWALL_RESET_IN_PROGRESS" = "1" ]; then
    echo "Setup failed after resetting UFW; attempting management-rule recovery" >&2
    if apply_management_ssh_rules && ufw --force enable; then
      echo "UFW was re-enabled with the configured management rules" >&2
    else
      echo "CRITICAL: automatic UFW recovery failed; repair it from the local console" >&2
    fi
  fi
  exit "$status"
}

trap on_exit EXIT

need_root() {
  if [ "$(id -u)" -ne 0 ]; then
    echo "Run as root, for example: sudo env YAP_LAN_SSH_CIDR=... bash setup-server.sh" >&2
    exit 1
  fi
}

valid_user() {
  printf '%s' "$1" | grep -Eq '^[a-z_][a-z0-9_-]*[$]?$'
}

valid_toggle() {
  [ "$1" = "0" ] || [ "$1" = "1" ]
}

select_validation_python() {
  for candidate in python3 python; do
    if command -v "$candidate" >/dev/null 2>&1 \
      && "$candidate" -c 'import ipaddress' >/dev/null 2>&1; then
      YAP_VALIDATION_PYTHON=$candidate
      return 0
    fi
  done
  die "Python with the standard-library ipaddress module is required for network validation"
}

valid_ip_network() {
  "$YAP_VALIDATION_PYTHON" - "$1" <<'PY'
import ipaddress
import sys

try:
    ipaddress.ip_network(sys.argv[1], strict=False)
except ValueError:
    raise SystemExit(1)
PY
}

valid_ip_interface() {
  "$YAP_VALIDATION_PYTHON" - "$1" <<'PY'
import ipaddress
import sys

try:
    ipaddress.ip_interface(sys.argv[1])
except ValueError:
    raise SystemExit(1)
PY
}

valid_ip_address() {
  "$YAP_VALIDATION_PYTHON" - "$1" <<'PY'
import ipaddress
import sys

try:
    ipaddress.ip_address(sys.argv[1])
except ValueError:
    raise SystemExit(1)
PY
}

policy_address_matches_management_source() {
  "$YAP_VALIDATION_PYTHON" - \
    "$YAP_SSH_POLICY_TEST_ADDR" \
    "$YAP_PRIVATE_SSH_FROM" \
    "$YAP_LAN_SSH_CIDR" \
    "$YAP_OVERLAY_SSH_CIDR" \
    "$YAP_ZSCALER_SSH_CIDR" <<'PY'
import ipaddress
import sys

address = ipaddress.ip_address(sys.argv[1])
sources = [ipaddress.ip_network(value, strict=False) for value in sys.argv[2:] if value]
if not sources or not any(address in source for source in sources):
    raise SystemExit(1)
PY
}

management_policy_addresses() {
  "$YAP_VALIDATION_PYTHON" - \
    "$YAP_SSH_POLICY_TEST_ADDR" \
    "$YAP_PRIVATE_SSH_FROM" \
    "$YAP_LAN_SSH_CIDR" \
    "$YAP_OVERLAY_SSH_CIDR" \
    "$YAP_ZSCALER_SSH_CIDR" <<'PY'
import ipaddress
import sys

addresses = []
seen = set()

def emit(address):
    value = str(address)
    if value not in seen:
        seen.add(value)
        addresses.append(value)

emit(ipaddress.ip_address(sys.argv[1]))
for value in sys.argv[2:]:
    if not value:
        continue
    network = ipaddress.ip_network(value, strict=False)
    candidate = network.network_address
    if network.num_addresses > 2:
        candidate += 1
    emit(candidate)

print("\n".join(addresses))
PY
}

validate_config() {
  valid_user "$YAP_OWNER" || die "Refusing invalid YAP_OWNER: $YAP_OWNER"
  select_validation_python

  case "$YAP_PRIVATE_IFACE" in
    ''|*[!a-zA-Z0-9_.:-]*) die "YAP_PRIVATE_IFACE contains unsupported characters" ;;
  esac
  valid_ip_interface "$YAP_PRIVATE_ADDR" \
    || die "YAP_PRIVATE_ADDR must be a valid IP interface with prefix length"

  for setting in \
    YAP_CONFIGURE_PRIVATE_ETHERNET \
    YAP_FIREWALL_RESET \
    YAP_DISABLE_NOISE_SERVICES \
    YAP_VALIDATE_ONLY; do
    value=${!setting}
    valid_toggle "$value" || die "$setting must be 0 or 1"
  done

  for setting in \
    YAP_PRIVATE_SSH_FROM \
    YAP_LAN_SSH_CIDR \
    YAP_OVERLAY_SSH_CIDR \
    YAP_ZSCALER_APP_CIDR \
    YAP_ZSCALER_SSH_CIDR \
    YAP_APP_CIDR; do
    value=${!setting}
    if [ -n "$value" ]; then
      valid_ip_network "$value" || die "$setting must be a valid IP address or CIDR"
    fi
  done

  valid_ip_address "$YAP_SSH_POLICY_TEST_ADDR" \
    || die "YAP_SSH_POLICY_TEST_ADDR must be one client IP address, not a CIDR"
  policy_address_matches_management_source \
    || die "YAP_SSH_POLICY_TEST_ADDR must fall within a configured SSH management source"

  for port in $YAP_TUNNEL_ONLY_PORTS; do
    case "$port" in
      *[!0-9]*) die "YAP_TUNNEL_ONLY_PORTS must contain only integer ports" ;;
    esac
    if [ "$port" -lt 1 ] || [ "$port" -gt 65535 ]; then
      die "YAP_TUNNEL_ONLY_PORTS must contain ports from 1 through 65535"
    fi
  done

  if [ -n "$YAP_APP_PORT" ]; then
    case "$YAP_APP_PORT" in
      *[!0-9]*) die "YAP_APP_PORT must be an integer from 1 through 65535" ;;
    esac
    if [ "$YAP_APP_PORT" -lt 1 ] || [ "$YAP_APP_PORT" -gt 65535 ]; then
      die "YAP_APP_PORT must be an integer from 1 through 65535"
    fi
    if [ -z "$YAP_APP_CIDR" ] && [ -z "$YAP_ZSCALER_APP_CIDR" ]; then
      die "YAP_APP_PORT is set, but no YAP_APP_CIDR or YAP_ZSCALER_APP_CIDR is set"
    fi
    for port in $YAP_TUNNEL_ONLY_PORTS; do
      if [ "$port" = "$YAP_APP_PORT" ]; then
        die "YAP_APP_PORT $YAP_APP_PORT is also listed in YAP_TUNNEL_ONLY_PORTS"
      fi
    done
  fi

  if [ "$YAP_CONFIGURE_PRIVATE_ETHERNET" = "1" ]; then
    command -v nmcli >/dev/null || die "nmcli is required when YAP_CONFIGURE_PRIVATE_ETHERNET=1"
  fi

  if [ "$YAP_FIREWALL_RESET" = "1" ]; then
    if [ -z "$YAP_PRIVATE_SSH_FROM" ] && \
      [ -z "$YAP_LAN_SSH_CIDR" ] && \
      [ -z "$YAP_OVERLAY_SSH_CIDR" ] && \
      [ -z "$YAP_ZSCALER_SSH_CIDR" ]; then
      die "YAP_FIREWALL_RESET=1 requires at least one explicit SSH allow source"
    fi
    [ "$YAP_FIREWALL_RESET_CONFIRM" = "local-console" ] \
      || die "YAP_FIREWALL_RESET=1 requires YAP_FIREWALL_RESET_CONFIRM=local-console"
  fi
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
  install -d -m 0755 /etc/ssh/sshd_config.d
  cat >/etc/ssh/sshd_config.d/98-yap-access.conf <<EOF
AuthenticationMethods publickey
AllowUsers ${YAP_OWNER}
AllowAgentForwarding no
PermitTunnel no
PermitUserEnvironment no
AllowTcpForwarding local
GatewayPorts no
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
  verify_effective_ssh_policy
  systemctl reload ssh
}

assert_effective_ssh_value() {
  policy=$1
  key=$2
  expected=$3
  actual=$(printf '%s\n' "$policy" | awk -v key="$key" '$1 == key { $1=""; sub(/^ /, ""); print; exit }')
  [ "$actual" = "$expected" ] \
    || die "Effective SSH policy mismatch for $key: expected '$expected', got '${actual:-<unset>}'"
}

verify_effective_ssh_policy() {
  policy_addresses=$(management_policy_addresses) \
    || die "Unable to derive representative SSH management addresses"
  [ -n "$policy_addresses" ] || die "No SSH management policy context is available"
  for policy_address in $policy_addresses; do
    criteria="user=${YAP_OWNER},host=yap-policy-check.invalid,addr=${policy_address}"
    owner_policy=$(sshd -T -C "$criteria") \
      || die "Unable to evaluate effective SSH policy for $criteria"
    assert_effective_ssh_value "$owner_policy" authenticationmethods publickey
    assert_effective_ssh_value "$owner_policy" allowusers "$YAP_OWNER"
    assert_effective_ssh_value "$owner_policy" allowagentforwarding no
    assert_effective_ssh_value "$owner_policy" permittunnel no
    assert_effective_ssh_value "$owner_policy" permituserenvironment no
    assert_effective_ssh_value "$owner_policy" allowtcpforwarding local
    assert_effective_ssh_value "$owner_policy" gatewayports no
    assert_effective_ssh_value "$owner_policy" permitrootlogin no
    assert_effective_ssh_value "$owner_policy" passwordauthentication no
    assert_effective_ssh_value "$owner_policy" pubkeyauthentication yes
    assert_effective_ssh_value "$owner_policy" kbdinteractiveauthentication no
    assert_effective_ssh_value "$owner_policy" x11forwarding no

    root_policy=$(sshd -T -C "user=root,host=yap-policy-check.invalid,addr=${policy_address}") \
      || die "Unable to evaluate effective SSH root policy for ${policy_address}"
    assert_effective_ssh_value "$root_policy" permitrootlogin no
    assert_effective_ssh_value "$root_policy" allowusers "$YAP_OWNER"
  done
}

setup_private_ethernet() {
  [ "$YAP_CONFIGURE_PRIVATE_ETHERNET" = "1" ] || return 0

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
  nmcli con up laptop-link
}

verify_private_management_address() {
  [ -n "$YAP_PRIVATE_SSH_FROM" ] || return 0
  command -v ip >/dev/null || die "ip is required to verify the private management interface"
  ip -4 -o address show dev "$YAP_PRIVATE_IFACE" 2>/dev/null \
    | awk '{print $4}' \
    | grep -Fxq "$YAP_PRIVATE_ADDR" \
    || die "Private management address $YAP_PRIVATE_ADDR is not active on $YAP_PRIVATE_IFACE"
}

apply_management_ssh_rules() {
  if [ -n "$YAP_PRIVATE_SSH_FROM" ]; then
    ufw allow in on "$YAP_PRIVATE_IFACE" from "$YAP_PRIVATE_SSH_FROM" to any port 22 proto tcp comment 'SSH from private management link' >/dev/null \
      || return 1
  fi
  if [ -n "$YAP_LAN_SSH_CIDR" ]; then
    ufw allow from "$YAP_LAN_SSH_CIDR" to any port 22 proto tcp comment 'SSH from LAN/VPN' >/dev/null \
      || return 1
  fi
  if [ -n "$YAP_OVERLAY_SSH_CIDR" ]; then
    ufw allow from "$YAP_OVERLAY_SSH_CIDR" to any port 22 proto tcp comment 'SSH from overlay' >/dev/null \
      || return 1
  fi
  if [ -n "$YAP_ZSCALER_SSH_CIDR" ]; then
    ufw allow from "$YAP_ZSCALER_SSH_CIDR" to any port 22 proto tcp comment 'SSH from Zscaler/ZPA' >/dev/null \
      || return 1
  fi
}

setup_firewall() {
  if [ "$YAP_FIREWALL_RESET" = "1" ]; then
    FIREWALL_RESET_IN_PROGRESS=1
    ufw --force reset >/dev/null
  fi
  ufw default deny incoming >/dev/null
  ufw default allow outgoing >/dev/null
  ufw default deny routed >/dev/null
  apply_management_ssh_rules

  if [ "$YAP_FIREWALL_RESET" = "1" ]; then
    ufw --force enable >/dev/null
    FIREWALL_RESET_IN_PROGRESS=0
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
    [ "$app_rules" -gt 0 ] || die "No application firewall rule was created"
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

main() {
  validate_config
  if [ "$YAP_VALIDATE_ONLY" = "1" ]; then
    echo "Yap server setup configuration is valid"
    return 0
  fi

  need_root
  install_basics
  setup_dirs
  setup_ssh
  setup_private_ethernet
  verify_private_management_address
  setup_firewall
  setup_logs
  setup_docker_logs
  disable_noise
  report
}

if [ -z "${BASH_SOURCE[0]:-}" ] || [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
