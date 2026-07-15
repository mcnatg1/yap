from __future__ import annotations

from concurrent.futures import Future
from dataclasses import dataclass
from datetime import datetime, timezone
from ipaddress import ip_address
import os
from pathlib import Path
import re
import stat
import threading
from typing import Mapping, Protocol

from yap_server.jobs.service import RecordingJobService
from yap_server.pools.batch_asr import (
    BatchAsrJob,
    BatchAsrPool,
    ContainerBatchAsrWorker,
    PoolBackpressure,
    inspect_worker_image,
)
from yap_server.pools.model_lock import load_model_pool_lock, verify_model_artifacts
from yap_server.workload_router import (
    RouterBackpressure,
    WorkloadRequest,
    WorkloadRouter,
)


PHASE5_BATCH_ENABLE = "YAP_PHASE5_BATCH_ENABLED"
_GIT_SHA = re.compile(r"^[0-9a-f]{40}$")


class BatchPool(Protocol):
    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]: ...


class RoutedBatchProcessor:
    """Small adapter that preserves the Phase 4 router-to-pool boundary."""

    def __init__(
        self,
        *,
        router: WorkloadRouter,
        pool: BatchPool,
        owner_key: str,
    ) -> None:
        self._router = router
        self._pool = pool
        self._owner_key = owner_key
        self._lock = threading.Lock()

    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        with self._lock:
            try:
                route = self._router.enqueue(
                    WorkloadRequest(
                        job_id=job.job_id,
                        owner_key=self._owner_key,
                        kind="batch",
                    )
                )
            except RouterBackpressure as error:
                raise PoolBackpressure("router capacity is unavailable") from error
            routed = self._router.dispatch(available_targets={"batch-asr"})
            if (
                route.target != "batch-asr"
                or routed is None
                or routed.request.job_id != job.job_id
                or routed.route.target != "batch-asr"
            ):
                raise RuntimeError("batch router dispatch identity is inconsistent")
            return self._pool.submit(job)


@dataclass(slots=True)
class BatchRuntime:
    service: RecordingJobService
    pool: BatchAsrPool

    def close(self) -> None:
        self.pool.shutdown()


def build_batch_runtime(
    environ: Mapping[str, str] | None = None,
    *,
    server_root: Path | None = None,
) -> BatchRuntime | None:
    source = os.environ if environ is None else environ
    enabled = source.get(PHASE5_BATCH_ENABLE, "0")
    if enabled == "0":
        return None
    if enabled != "1":
        raise ValueError(f"{PHASE5_BATCH_ENABLE} must be 0 or 1")
    if os.name != "posix" or not hasattr(os, "getuid") or not hasattr(os, "getgid"):
        raise ValueError("the Phase 5 GPU runtime requires the Linux server node")
    run_as_uid = os.getuid()
    run_as_gid = os.getgid()
    if run_as_uid < 1 or run_as_gid < 1:
        raise ValueError("the Phase 5 server process must run as a non-root account")

    root = (
        server_root.resolve()
        if server_root is not None
        else Path(__file__).resolve().parents[3]
    )
    lock_path = Path(
        source.get("YAP_PHASE5_MODEL_LOCK", str(root / "model-pools.lock.json"))
    ).resolve(strict=True)
    model_dir = _required_existing_directory(source, "YAP_PHASE5_MODEL_DIR")
    storage_dir = _private_storage_directory(source, "YAP_PHASE5_STORAGE_DIR")
    timeout_seconds = _positive_float(
        source.get("YAP_PHASE5_WORKER_TIMEOUT_SECONDS", "1800"),
        "YAP_PHASE5_WORKER_TIMEOUT_SECONDS",
    )
    lock = load_model_pool_lock(lock_path)
    verify_model_artifacts(lock, model_dir)
    docker_binary = source.get("YAP_PHASE5_DOCKER_BINARY", "docker")
    worker_image = resolve_phase5_worker_image(
        source,
        docker_binary=docker_binary,
    )
    worker = ContainerBatchAsrWorker(
        image=worker_image,
        model_dir=model_dir,
        lock=lock,
        run_as_uid=run_as_uid,
        run_as_gid=run_as_gid,
        docker_binary=docker_binary,
        timeout_seconds=timeout_seconds,
    )
    pool = BatchAsrPool(worker, max_workers=1, max_queued=2)
    router = WorkloadRouter(
        max_pending=3,
        max_pending_per_owner=3,
        max_consecutive_live=8,
    )
    processor = RoutedBatchProcessor(
        router=router,
        pool=pool,
        owner_key="development-loopback",
    )
    service = RecordingJobService(
        storage_dir,
        processor=processor,
        supported_languages=lock.supported_languages,
        now=_utc_now,
    )
    return BatchRuntime(service=service, pool=pool)


def ensure_development_batch_bind(host: str) -> None:
    try:
        if ip_address(host).is_loopback:
            return
    except ValueError:
        pass
    raise ValueError(
        "Phase 5 batch audio is development-only and must bind to loopback; "
        "use an SSH tunnel until Phase 7 authentication and certificate policy ship"
    )


def resolve_phase5_worker_image(
    environ: Mapping[str, str],
    *,
    docker_binary: str,
) -> str:
    image = environ.get("YAP_PHASE5_WORKER_IMAGE", "").strip()
    checked_head = environ.get("YAP_PHASE5_CHECKED_HEAD", "").strip()
    if not image or _GIT_SHA.fullmatch(checked_head) is None:
        raise ValueError(
            "YAP_PHASE5_WORKER_IMAGE and a full YAP_PHASE5_CHECKED_HEAD are required"
        )
    try:
        inspected = inspect_worker_image(
            image,
            checked_head,
            docker_binary=docker_binary,
        )
    except RuntimeError as error:
        raise ValueError(str(error)) from None
    image_id = inspected.get("id")
    if not isinstance(image_id, str):
        raise ValueError("checked-head worker image inspection omitted its immutable ID")
    return image_id


def _required_existing_directory(
    environ: Mapping[str, str],
    name: str,
) -> Path:
    value = environ.get(name, "").strip()
    if not value:
        raise ValueError(f"{name} is required when Phase 5 batch is enabled")
    path = Path(value).resolve(strict=True)
    if not path.is_dir():
        raise ValueError(f"{name} must be a directory")
    return path


def _private_storage_directory(
    environ: Mapping[str, str],
    name: str,
) -> Path:
    value = environ.get(name, "").strip()
    if not value:
        raise ValueError(f"{name} is required when Phase 5 batch is enabled")
    requested = Path(value)
    requested.mkdir(parents=True, mode=0o700, exist_ok=True)
    metadata = requested.lstat()
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
        raise ValueError(f"{name} must be a real directory")
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise ValueError(f"{name} must not grant group or other permissions")
    return requested.resolve(strict=True)


def _positive_float(value: str, name: str) -> float:
    try:
        parsed = float(value)
    except ValueError as error:
        raise ValueError(f"{name} must be numeric") from error
    if parsed <= 0:
        raise ValueError(f"{name} must be positive")
    return parsed


def _utc_now() -> str:
    return (
        datetime.now(timezone.utc)
        .isoformat(timespec="milliseconds")
        .replace("+00:00", "Z")
    )
