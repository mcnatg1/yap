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

lock_path="$repo_root/server/model-pools.lock.json"
image="yap-phase4-asr:phase4-$YAP_CHECKED_HEAD"
mkdir -p "$YAP_PHASE4_MODEL_DIR" "$YAP_PHASE4_EVIDENCE_DIR"

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
    --result "$YAP_PHASE4_EVIDENCE_DIR/result.json" \
    --evidence "$YAP_PHASE4_EVIDENCE_DIR/evidence.json"
