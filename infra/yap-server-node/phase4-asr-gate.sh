#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/../.." && pwd)"

: "${YAP_CHECKED_HEAD:?Set YAP_CHECKED_HEAD to the exact 40-character candidate SHA}"
: "${YAP_PHASE4_MODEL_DIR:=/srv/yap-server/models/cohere-transcribe-03-2026/b1eacc2686a3d08ceaae5f24a88b1d519620bc09}"
: "${YAP_PHASE4_EVIDENCE_DIR:=/srv/yap-server/shared/phase4-evidence/$YAP_CHECKED_HEAD}"

if [[ ! "$YAP_CHECKED_HEAD" =~ ^[0-9a-f]{40}$ ]]; then
  echo "YAP_CHECKED_HEAD must be a full lowercase Git SHA" >&2
  exit 2
fi

if ! inside_worktree="$(
  git -C "$repo_root" rev-parse --is-inside-work-tree 2>/dev/null
)" || [ "$inside_worktree" != "true" ]; then
  echo "Phase 4 gate requires a Git worktree" >&2
  exit 2
fi

actual_head="$(git -C "$repo_root" rev-parse HEAD)"
if [ "$actual_head" != "$YAP_CHECKED_HEAD" ]; then
  echo "checked head does not match the repository HEAD" >&2
  exit 2
fi
worktree_status="$(
  git -C "$repo_root" status --porcelain=v1 --untracked-files=normal
)"
if [ -n "$worktree_status" ]; then
  echo "Phase 4 gate requires a clean checked head" >&2
  exit 2
fi

capture_host_boundary() {
  local target="$1"
  mkdir -p "$target"

  if ! command -v ss >/dev/null 2>&1; then
    echo "Phase 4 gate requires ss for listener read-back" >&2
    return 1
  fi
  ss -H -lntu | LC_ALL=C sort >"$target/listeners.txt"

  if command -v ufw >/dev/null 2>&1; then
    {
      printf '%s\n' "tool=ufw"
      sudo -n ufw status verbose
    } >"$target/firewall.txt"
  elif command -v nft >/dev/null 2>&1; then
    {
      printf '%s\n' "tool=nft"
      sudo -n nft list ruleset
    } >"$target/firewall.txt"
  elif command -v iptables-save >/dev/null 2>&1; then
    {
      printf '%s\n' "tool=iptables-save"
      sudo -n iptables-save
    } >"$target/firewall.txt"
  else
    printf '%s\n' "tool=none" >"$target/firewall.txt"
  fi

  if command -v systemctl >/dev/null 2>&1; then
    systemctl list-unit-files --type=service --no-legend --no-pager \
      | awk '$1 ~ /^yap.*\.service$/ { print }' \
      | LC_ALL=C sort >"$target/services.txt"
  else
    printf '%s\n' "systemd-unavailable" >"$target/services.txt"
  fi

  docker ps -a --format '{{.Names}}' \
    | awk '/^yap-phase4-asr-[0-9a-f]+$/ { print }' \
    | LC_ALL=C sort >"$target/containers.txt"
  (pgrep -af '[y]ap_server\.pools\.batch_asr_worker' || true) \
    | LC_ALL=C sort >"$target/workers.txt"
}

lock_path="$repo_root/server/model-pools.lock.json"
image="yap-phase4-asr:phase4-$YAP_CHECKED_HEAD"
mkdir -p "$YAP_PHASE4_MODEL_DIR" "$YAP_PHASE4_EVIDENCE_DIR"
gate_tmp="$(mktemp -d "${TMPDIR:-/tmp}/yap-phase4-gate.XXXXXXXX")"
trap 'rm -rf -- "$gate_tmp"' EXIT

capture_host_boundary "$gate_tmp/before"

PYTHONPATH="$repo_root/server/src" \
  python3 -m yap_server.pools.model_assets \
    --lock "$lock_path" \
    --model-dir "$YAP_PHASE4_MODEL_DIR"

docker build \
  --pull \
  --file "$repo_root/server/runtime/asr/Dockerfile" \
  --label "org.opencontainers.image.revision=$YAP_CHECKED_HEAD" \
  --tag "$image" \
  "$repo_root/server"

PYTHONPATH="$repo_root/server/src" \
  python3 -m yap_server.pools.phase4_gate \
    --checked-head "$YAP_CHECKED_HEAD" \
    --image "$image" \
    --lock "$lock_path" \
    --model-dir "$YAP_PHASE4_MODEL_DIR" \
    --repo-root "$repo_root" \
    --result "$gate_tmp/inference-result.json" \
    --evidence "$gate_tmp/inference-evidence.json"

capture_host_boundary "$gate_tmp/after"

PYTHONPATH="$repo_root/server/src" \
  python3 -m yap_server.pools.phase4_evidence \
    --before "$gate_tmp/before" \
    --after "$gate_tmp/after" \
    --inference-result "$gate_tmp/inference-result.json" \
    --inference-evidence "$gate_tmp/inference-evidence.json" \
    --result "$YAP_PHASE4_EVIDENCE_DIR/result.json" \
    --evidence "$YAP_PHASE4_EVIDENCE_DIR/evidence.json"
