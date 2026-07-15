from __future__ import annotations

import json
from pathlib import Path
import re
import subprocess
import threading
from typing import Callable
from uuid import uuid4

from yap_server.pools.batch_contract import (
    BatchAsrJob,
    BatchWorker,
    DuplicatePoolJob,
    PoolBackpressure,
    PoolFenced,
    WorkerContainmentError,
    WorkerExecutionError,
)
from yap_server.pools.batch_pool import BatchAsrPool
from yap_server.pools.batch_result import (
    publish_result as _publish_result,
    validate_result as _validate_result,
)
from yap_server.pools.container_runtime import (
    CONTAINER_LABEL_VALUE as _CONTAINER_LABEL_VALUE,
    JOB_LABEL as _JOB_LABEL,
    MAX_WORKER_OUTPUT_BYTES as _MAX_WORKER_OUTPUT_BYTES,
    OWNER_LABEL as _OWNER_LABEL,
    OWNER_VALUE as _OWNER_VALUE,
    REVISION_LABEL as _REVISION_LABEL,
    RUNTIME_LABEL as _RUNTIME_LABEL,
    STORAGE_LABEL as _STORAGE_LABEL,
    force_remove_container as _force_remove_container,
    reconcile_owned_containers,
    run_bounded_process as _run_bounded_process,
    validate_worker_output as _validate_worker_output,
)
from yap_server.pools.model_lock import ModelPoolLock


_GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
_IMMUTABLE_IMAGE = re.compile(r"^(?:sha256:[0-9a-f]{64}|.+@sha256:[0-9a-f]{64})$")
_WORKER_MEMORY_LIMIT = "96g"
_WORKER_CPU_LIMIT = "16"


def inspect_worker_image(
    image: str,
    checked_head: str,
    *,
    docker_binary: str = "docker",
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> dict[str, object]:
    if not image.strip() or _GIT_SHA.fullmatch(checked_head) is None:
        raise ValueError("worker image and checked head are required")
    completed = runner(
        [docker_binary, "image", "inspect", image],
        check=False,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
        stdin=subprocess.DEVNULL,
    )
    if completed.returncode != 0:
        raise RuntimeError("could not inspect the checked-head worker image")
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("worker image inspection returned invalid JSON") from error
    if not isinstance(payload, list) or len(payload) != 1 or not isinstance(payload[0], dict):
        raise RuntimeError("worker image inspection returned an unexpected shape")
    record = payload[0]
    config = record.get("Config")
    labels = config.get("Labels") if isinstance(config, dict) else None
    if (
        not isinstance(labels, dict)
        or labels.get("org.opencontainers.image.revision") != checked_head
    ):
        raise RuntimeError("worker image revision label does not match the checked head")
    image_id = record.get("Id")
    architecture = record.get("Architecture")
    repo_digests = record.get("RepoDigests")
    if not isinstance(image_id, str) or not _IMMUTABLE_IMAGE.fullmatch(image_id):
        raise RuntimeError("worker image ID is invalid")
    if architecture != "arm64":
        raise RuntimeError("worker image architecture is not ARM64")
    if repo_digests is None:
        repo_digests = []
    if not isinstance(repo_digests, list) or not all(
        isinstance(item, str) for item in repo_digests
    ):
        raise RuntimeError("worker image repository digests are invalid")
    return {
        "reference": image,
        "id": image_id,
        "architecture": architecture,
        "repoDigests": repo_digests,
        "revision": checked_head,
    }


class ContainerBatchAsrWorker:
    def __init__(
        self,
        *,
        image: str,
        model_dir: Path,
        lock: ModelPoolLock,
        run_as_uid: int,
        run_as_gid: int,
        checked_head: str,
        storage_namespace: str,
        runtime_instance_id: str | None = None,
        docker_binary: str = "docker",
        timeout_seconds: float = 30 * 60,
        runner: Callable[..., subprocess.CompletedProcess[str]] | None = None,
    ) -> None:
        if not _is_pinned_image(image):
            raise ValueError("worker image must use an immutable image ID or digest")
        if timeout_seconds <= 0:
            raise ValueError("worker timeout must be positive")
        if _GIT_SHA.fullmatch(checked_head) is None:
            raise ValueError("worker checked head must be a full lowercase Git SHA")
        if _CONTAINER_LABEL_VALUE.fullmatch(storage_namespace) is None:
            raise ValueError("worker storage namespace is invalid")
        resolved_runtime_id = runtime_instance_id or uuid4().hex
        if _CONTAINER_LABEL_VALUE.fullmatch(resolved_runtime_id) is None:
            raise ValueError("worker runtime instance ID is invalid")
        if (
            not isinstance(run_as_uid, int)
            or isinstance(run_as_uid, bool)
            or run_as_uid < 1
            or not isinstance(run_as_gid, int)
            or isinstance(run_as_gid, bool)
            or run_as_gid < 1
        ):
            raise ValueError("worker identity must be an explicit non-root UID and GID")
        self._image = image
        self._lock = lock
        self._run_as_identity = f"{run_as_uid}:{run_as_gid}"
        self._run_as_uid = run_as_uid
        self._run_as_gid = run_as_gid
        self._checked_head = checked_head
        self._storage_namespace = storage_namespace
        self._runtime_instance_id = resolved_runtime_id
        self._model_dir = _safe_mount_path(model_dir.resolve(strict=True))
        if not self._model_dir.is_dir():
            raise ValueError("model_dir must be a directory")
        self._docker_binary = docker_binary
        self._timeout_seconds = timeout_seconds
        self._runner = runner
        self._shutdown = threading.Event()

    def close(self) -> None:
        self._shutdown.set()

    def build_command(self, job: BatchAsrJob) -> list[str]:
        return self._build_command(
            job,
            container_name=f"yap-phase4-asr-{uuid4().hex}",
        )

    def _build_command(
        self,
        job: BatchAsrJob,
        *,
        container_name: str,
    ) -> list[str]:
        if job.language not in self._lock.supported_languages:
            raise ValueError("batch language is not supported by the locked model")
        input_path = _safe_mount_path(job.input_path.resolve(strict=True))
        if not input_path.is_file():
            raise ValueError("batch input must be a regular file")
        return [
            self._docker_binary,
            "run",
            "--rm",
            "--name",
            container_name,
            "--label",
            f"{_OWNER_LABEL}={_OWNER_VALUE}",
            "--label",
            f"{_STORAGE_LABEL}={self._storage_namespace}",
            "--label",
            f"{_RUNTIME_LABEL}={self._runtime_instance_id}",
            "--label",
            f"{_JOB_LABEL}={job.job_id}",
            "--label",
            f"{_REVISION_LABEL}={self._checked_head}",
            "--pull",
            "never",
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--user",
            self._run_as_identity,
            "--pids-limit",
            "512",
            "--memory",
            _WORKER_MEMORY_LIMIT,
            "--memory-swap",
            _WORKER_MEMORY_LIMIT,
            "--cpus",
            _WORKER_CPU_LIMIT,
            "--shm-size",
            "1g",
            "--tmpfs",
            "/tmp:rw,nosuid,nodev,noexec,size=1g",
            "--tmpfs",
            (
                "/triton-cache:rw,nosuid,nodev,exec,size=256m,mode=0700,"
                f"uid={self._run_as_uid},gid={self._run_as_gid}"
            ),
            "--device",
            "nvidia.com/gpu=all",
            "--env",
            "TRITON_CACHE_DIR=/triton-cache",
            "--env",
            "HF_HUB_OFFLINE=1",
            "--env",
            "TRANSFORMERS_OFFLINE=1",
            "--env",
            "HF_HUB_DISABLE_TELEMETRY=1",
            "--env",
            "DO_NOT_TRACK=1",
            "--mount",
            f"type=bind,src={self._model_dir},dst=/models/asr,readonly",
            "--mount",
            f"type=bind,src={input_path},dst=/input/audio.wav,readonly",
            self._image,
            "--model-dir",
            "/models/asr",
            "--input",
            "/input/audio.wav",
            "--job-id",
            job.job_id,
            "--language",
            job.language,
            *([] if job.punctuation else ["--no-punctuation"]),
        ]

    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event | None = None,
    ) -> dict[str, object]:
        job_cancellation = cancellation or threading.Event()
        if self._shutdown.is_set() or job_cancellation.is_set():
            raise WorkerExecutionError("isolated ASR worker was cancelled")
        container_name = f"yap-phase4-asr-{uuid4().hex}"
        command = self._build_command(job, container_name=container_name)
        if self._runner is None:
            try:
                completed = _run_bounded_process(
                    command,
                    timeout_seconds=self._timeout_seconds,
                    output_limit_bytes=_MAX_WORKER_OUTPUT_BYTES,
                    cancellation=(self._shutdown, job_cancellation),
                )
            finally:
                _force_remove_container(self._docker_binary, container_name)
        else:
            completed = self._runner(
                command,
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=self._timeout_seconds,
                stdin=subprocess.DEVNULL,
            )
        _validate_worker_output(completed)
        if completed.returncode != 0:
            raise WorkerExecutionError(
                f"isolated ASR worker exited with status {completed.returncode}"
            )
        try:
            payload = json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise WorkerExecutionError("isolated ASR worker returned invalid JSON") from error
        _validate_result(payload, job, self._lock)
        result = dict(payload)
        _publish_result(job.result_path, result)
        return result


def _is_pinned_image(image: str) -> bool:
    return _IMMUTABLE_IMAGE.fullmatch(image) is not None


def _safe_mount_path(path: Path) -> Path:
    text = str(path)
    if any(character in text for character in (",", "\n", "\r")):
        raise ValueError("container mount paths cannot contain commas or newlines")
    return path
