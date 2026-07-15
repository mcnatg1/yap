#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/../.." && pwd)"

: "${YAP_CHECKED_HEAD:?Set YAP_CHECKED_HEAD to the exact 40-character candidate SHA}"
: "${YAP_PHASE5_MODEL_DIR:?Set YAP_PHASE5_MODEL_DIR to the verified private model directory}"
: "${YAP_PHASE5_STORAGE_DIR:?Set YAP_PHASE5_STORAGE_DIR to a private job directory}"
: "${YAP_PHASE5_WORKER_IMAGE:?Set YAP_PHASE5_WORKER_IMAGE to the checked-head Yap worker image}"
: "${YAP_PHASE5_MODEL_LOCK:=$repo_root/server/model-pools.lock.json}"
: "${YAP_PHASE5_WORKER_TIMEOUT_SECONDS:=1800}"

if [[ ! "$YAP_CHECKED_HEAD" =~ ^[0-9a-f]{40}$ ]]; then
  echo "YAP_CHECKED_HEAD must be a full lowercase Git SHA" >&2
  exit 2
fi

if ! inside_worktree="$(
  git -C "$repo_root" rev-parse --is-inside-work-tree 2>/dev/null
)" || [ "$inside_worktree" != "true" ]; then
  echo "Phase 5 server launch requires a Git worktree" >&2
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
  echo "Phase 5 server launch requires a clean checked head" >&2
  exit 2
fi

if [ ! -d "$YAP_PHASE5_MODEL_DIR" ]; then
  echo "YAP_PHASE5_MODEL_DIR must be an existing directory" >&2
  exit 2
fi
if [ ! -f "$YAP_PHASE5_MODEL_LOCK" ]; then
  echo "YAP_PHASE5_MODEL_LOCK must be an existing file" >&2
  exit 2
fi
if ! command -v python3.12 >/dev/null 2>&1; then
  echo "Phase 5 requires python3.12" >&2
  exit 2
fi
python_version="$(python3.12 -c 'import sys; print(".".join(map(str, sys.version_info[:2])))')"
if [ "$python_version" != "3.12" ]; then
  echo "Phase 5 requires Python 3.12" >&2
  exit 2
fi

umask 077
mkdir -p -- "$YAP_PHASE5_STORAGE_DIR"
if [ -L "$YAP_PHASE5_STORAGE_DIR" ] || [ ! -d "$YAP_PHASE5_STORAGE_DIR" ]; then
  echo "YAP_PHASE5_STORAGE_DIR must be a real directory" >&2
  exit 2
fi
storage_mode="$(stat -Lc '%a' "$YAP_PHASE5_STORAGE_DIR")"
if [ "$storage_mode" != "700" ]; then
  echo "YAP_PHASE5_STORAGE_DIR must have mode 0700" >&2
  exit 2
fi

exec env \
  PYTHONNOUSERSITE=1 \
  PYTHONPATH="$repo_root/server/src" \
  YAP_SERVER_HOST=127.0.0.1 \
  YAP_SERVER_PORT=18765 \
  YAP_PHASE5_BATCH_ENABLED=1 \
  YAP_PHASE5_CHECKED_HEAD="$YAP_CHECKED_HEAD" \
  YAP_PHASE5_WORKER_IMAGE="$YAP_PHASE5_WORKER_IMAGE" \
  YAP_PHASE5_MODEL_LOCK="$YAP_PHASE5_MODEL_LOCK" \
  YAP_PHASE5_MODEL_DIR="$YAP_PHASE5_MODEL_DIR" \
  YAP_PHASE5_STORAGE_DIR="$YAP_PHASE5_STORAGE_DIR" \
  YAP_PHASE5_WORKER_TIMEOUT_SECONDS="$YAP_PHASE5_WORKER_TIMEOUT_SECONDS" \
  python3.12 -m yap_server
