from __future__ import annotations

from concurrent.futures import Future, ThreadPoolExecutor
from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time
from typing import BinaryIO, Callable, Protocol
from uuid import uuid4

from yap_server.pools.model_lock import ModelPoolLock


_JOB_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
_LANGUAGE = re.compile(r"^[a-z]{2}$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
_IMMUTABLE_IMAGE = re.compile(r"^(?:sha256:[0-9a-f]{64}|.+@sha256:[0-9a-f]{64})$")
_WORKER_MEMORY_LIMIT = "96g"
_WORKER_CPU_LIMIT = "16"
_MAX_WORKER_OUTPUT_BYTES = 1024 * 1024
_PROCESS_READ_BYTES = 64 * 1024
_CONTAINER_CLEANUP_TIMEOUT_SECONDS = 30
_CONTAINER_ID = re.compile(r"^[0-9a-f]{12,64}$")
_CONTAINER_LABEL_VALUE = re.compile(r"^[A-Za-z0-9._-]{1,64}$")
_OWNER_LABEL = "com.mcnatg1.yap.owner"
_OWNER_VALUE = "batch-asr"
_STORAGE_LABEL = "com.mcnatg1.yap.storage"
_RUNTIME_LABEL = "com.mcnatg1.yap.runtime"
_JOB_LABEL = "com.mcnatg1.yap.job"
_REVISION_LABEL = "org.opencontainers.image.revision"


class PoolBackpressure(RuntimeError):
    """Raised when every worker and bounded queue slot is occupied."""


class PoolFenced(PoolBackpressure):
    """Raised when worker containment is uncertain and capacity is quarantined."""


class DuplicatePoolJob(ValueError):
    """Raised when a job is already running or queued in the pool."""


class WorkerExecutionError(RuntimeError):
    """Raised when the isolated GPU worker fails or returns invalid output."""


class WorkerContainmentError(WorkerExecutionError):
    """Raised when an owned worker container cannot be proven absent."""


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


@dataclass(frozen=True)
class BatchAsrJob:
    job_id: str
    input_path: Path
    result_path: Path
    language: str
    input_sha256: str
    punctuation: bool = True

    def __post_init__(self) -> None:
        if not _JOB_ID.fullmatch(self.job_id):
            raise ValueError("job_id must be an opaque path-safe identifier")
        if not _LANGUAGE.fullmatch(self.language):
            raise ValueError("language must be an explicit lowercase ISO 639-1 code")
        if not _SHA256.fullmatch(self.input_sha256):
            raise ValueError("input_sha256 must be a lowercase SHA-256 digest")


class BatchWorker(Protocol):
    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]: ...


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


def _force_remove_container(
    docker_binary: str,
    container_name: str,
    *,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> None:
    try:
        completed = runner(
            [docker_binary, "container", "rm", "--force", container_name],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=_CONTAINER_CLEANUP_TIMEOUT_SECONDS,
            stdin=subprocess.DEVNULL,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise WorkerContainmentError(
            "could not verify isolated ASR container cleanup"
        ) from error
    stderr = completed.stderr if isinstance(completed.stderr, str) else ""
    missing = any(
        marker in stderr.casefold()
        for marker in ("no such container", "no such object")
    )
    if completed.returncode != 0 and not missing:
        raise WorkerContainmentError("could not remove the isolated ASR container")


def reconcile_owned_containers(
    docker_binary: str,
    *,
    storage_namespace: str,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> int:
    if _CONTAINER_LABEL_VALUE.fullmatch(storage_namespace) is None:
        raise ValueError("container storage namespace is invalid")
    try:
        completed = runner(
            [
                docker_binary,
                "container",
                "ls",
                "--all",
                "--quiet",
                "--filter",
                f"label={_OWNER_LABEL}={_OWNER_VALUE}",
                "--filter",
                f"label={_STORAGE_LABEL}={storage_namespace}",
            ],
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=_CONTAINER_CLEANUP_TIMEOUT_SECONDS,
            stdin=subprocess.DEVNULL,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise WorkerContainmentError(
            "could not inspect owned ASR containers during startup"
        ) from error
    if completed.returncode != 0:
        raise WorkerContainmentError(
            "could not inspect owned ASR containers during startup"
        )
    container_ids = [
        line.strip() for line in completed.stdout.splitlines() if line.strip()
    ]
    if not all(_CONTAINER_ID.fullmatch(container_id) for container_id in container_ids):
        raise WorkerContainmentError("owned ASR container inventory was invalid")
    for container_id in container_ids:
        _force_remove_container(docker_binary, container_id, runner=runner)
    return len(container_ids)


def _validate_worker_output(completed: subprocess.CompletedProcess[str]) -> None:
    for stream_name, value in (
        ("stdout", completed.stdout),
        ("stderr", completed.stderr),
    ):
        if not isinstance(value, str):
            raise WorkerExecutionError(
                f"isolated ASR worker {stream_name} was not decoded text"
            )
        if len(value.encode("utf-8", errors="replace")) > _MAX_WORKER_OUTPUT_BYTES:
            raise WorkerExecutionError(
                f"isolated ASR worker {stream_name} exceeded the bounded output"
            )


def _run_bounded_process(
    command: list[str],
    *,
    timeout_seconds: float,
    output_limit_bytes: int,
    cancellation: threading.Event | tuple[threading.Event, ...] | None = None,
) -> subprocess.CompletedProcess[str]:
    process = subprocess.Popen(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if process.stdout is None or process.stderr is None:
        process.kill()
        process.wait()
        raise WorkerExecutionError("isolated ASR worker pipes were not created")

    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    exceeded = threading.Event()
    kill_lock = threading.Lock()

    def kill_process() -> None:
        with kill_lock:
            if process.poll() is None:
                process.kill()

    def read_stream(name: str, stream: BinaryIO) -> None:
        try:
            while True:
                block = stream.read(_PROCESS_READ_BYTES)
                if not block:
                    break
                remaining = output_limit_bytes - len(buffers[name])
                if len(block) > remaining:
                    if remaining > 0:
                        buffers[name].extend(block[:remaining])
                    exceeded.set()
                    kill_process()
                    break
                buffers[name].extend(block)
        finally:
            stream.close()

    readers = [
        threading.Thread(
            target=read_stream,
            args=("stdout", process.stdout),
            name="yap-asr-stdout",
            daemon=True,
        ),
        threading.Thread(
            target=read_stream,
            args=("stderr", process.stderr),
            name="yap-asr-stderr",
            daemon=True,
        ),
    ]
    for reader in readers:
        reader.start()
    deadline = time.monotonic() + timeout_seconds
    while True:
        cancellation_requested = (
            cancellation.is_set()
            if isinstance(cancellation, threading.Event)
            else any(event.is_set() for event in cancellation or ())
        )
        if cancellation_requested:
            kill_process()
            process.wait()
            for reader in readers:
                reader.join()
            raise WorkerExecutionError("isolated ASR worker was cancelled")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            kill_process()
            process.wait()
            for reader in readers:
                reader.join()
            raise WorkerExecutionError("isolated ASR worker timed out")
        try:
            returncode = process.wait(timeout=min(0.1, remaining))
            break
        except subprocess.TimeoutExpired:
            continue
    for reader in readers:
        reader.join()
    if exceeded.is_set():
        raise WorkerExecutionError("isolated ASR worker exceeded the bounded output")
    return subprocess.CompletedProcess(
        args=command,
        returncode=returncode,
        stdout=bytes(buffers["stdout"]).decode("utf-8", errors="replace"),
        stderr=bytes(buffers["stderr"]).decode("utf-8", errors="replace"),
    )


class BatchAsrPool:
    """A bounded thread-backed pool for isolated batch-ASR workers."""

    def __init__(
        self,
        worker: BatchWorker,
        *,
        max_workers: int = 1,
        max_queued: int = 2,
    ) -> None:
        if max_workers < 1 or max_queued < 0:
            raise ValueError("pool limits are invalid")
        self._worker = worker
        self._slots = threading.BoundedSemaphore(max_workers + max_queued)
        self._lock = threading.Lock()
        self._outstanding: set[str] = set()
        self._cancellations: dict[str, threading.Event] = {}
        self._futures: dict[str, Future[dict[str, object]]] = {}
        self._fenced_reason: str | None = None
        self._executor = ThreadPoolExecutor(
            max_workers=max_workers,
            thread_name_prefix="yap-batch-asr",
        )

    @property
    def outstanding_count(self) -> int:
        with self._lock:
            return len(self._outstanding)

    @property
    def fenced(self) -> bool:
        with self._lock:
            return self._fenced_reason is not None

    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        with self._lock:
            if self._fenced_reason is not None:
                raise PoolFenced(self._fenced_reason)
            if job.job_id in self._outstanding:
                raise DuplicatePoolJob(f"pool job {job.job_id!r} is already outstanding")
            if not self._slots.acquire(blocking=False):
                raise PoolBackpressure("batch ASR pool is at its bounded capacity")
            self._outstanding.add(job.job_id)
            cancellation = threading.Event()
            self._cancellations[job.job_id] = cancellation
        try:
            future = self._executor.submit(self._run_job, job, cancellation)
            with self._lock:
                self._futures[job.job_id] = future
        except BaseException:
            self._release(job.job_id)
            raise
        future.add_done_callback(lambda _future: self._release(job.job_id))
        return future

    def _run_job(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        try:
            return self._worker.run(job, cancellation)
        except WorkerContainmentError:
            with self._lock:
                self._fenced_reason = (
                    "batch ASR pool is fenced because container cleanup "
                    "could not be verified"
                )
            raise

    def cancel(self, job_id: str) -> bool:
        with self._lock:
            cancellation = self._cancellations.get(job_id)
            future = self._futures.get(job_id)
            if cancellation is None or future is None:
                return False
            cancellation.set()
        future.cancel()
        return True

    def _release(self, job_id: str) -> None:
        with self._lock:
            self._outstanding.discard(job_id)
            self._cancellations.pop(job_id, None)
            self._futures.pop(job_id, None)
            self._slots.release()

    def shutdown(self) -> None:
        close_worker = getattr(self._worker, "close", None)
        try:
            with self._lock:
                for cancellation in self._cancellations.values():
                    cancellation.set()
            if callable(close_worker):
                close_worker()
        finally:
            self._executor.shutdown(wait=True, cancel_futures=True)


def _is_pinned_image(image: str) -> bool:
    return _IMMUTABLE_IMAGE.fullmatch(image) is not None


def _safe_mount_path(path: Path) -> Path:
    text = str(path)
    if any(character in text for character in (",", "\n", "\r")):
        raise ValueError("container mount paths cannot contain commas or newlines")
    return path


def _validate_result(
    payload: object,
    job: BatchAsrJob,
    lock: ModelPoolLock,
) -> None:
    if not isinstance(payload, dict):
        raise WorkerExecutionError("isolated ASR worker result must be an object")
    if payload.get("schemaVersion") != 1 or payload.get("jobId") != job.job_id:
        raise WorkerExecutionError("isolated ASR worker result identity is invalid")
    model = payload.get("model")
    if not isinstance(model, dict) or (
        model.get("poolId") != lock.pool_id
        or model.get("id") != lock.model_id
        or model.get("revision") != lock.model_revision
    ):
        raise WorkerExecutionError("isolated ASR worker model identity is invalid")
    audio = payload.get("audio")
    duration_ms = audio.get("durationMs") if isinstance(audio, dict) else None
    if (
        not isinstance(audio, dict)
        or audio.get("sha256") != job.input_sha256
        or audio.get("sampleRateHz") != 16000
        or not isinstance(duration_ms, int)
        or isinstance(duration_ms, bool)
        or duration_ms <= 0
    ):
        raise WorkerExecutionError("isolated ASR worker audio identity is invalid")
    transcript = payload.get("transcript")
    if (
        not isinstance(transcript, dict)
        or not isinstance(transcript.get("text"), str)
        or not transcript["text"].strip()
        or transcript.get("language") != job.language
        or transcript.get("punctuation") is not job.punctuation
    ):
        raise WorkerExecutionError("isolated ASR worker transcript is invalid")
    runtime = payload.get("runtime")
    python_version = runtime.get("pythonVersion") if isinstance(runtime, dict) else None
    if (
        not isinstance(runtime, dict)
        or runtime.get("device") != "cuda"
        or runtime.get("torchVersion") != lock.runtime_torch_version
        or runtime.get("torchCudaVersion") != lock.runtime_torch_cuda_version
        or runtime.get("overlayPackages") != dict(lock.runtime_overlay_packages)
        or not isinstance(python_version, str)
        or python_version.split(".")[:2] != lock.runtime_python_version.split(".")
    ):
        raise WorkerExecutionError("isolated ASR worker runtime identity is invalid")


def _publish_result(path: Path, payload: dict[str, object]) -> None:
    destination = path.resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary_name: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            prefix=f".{destination.name}.",
            suffix=".tmp",
            dir=destination.parent,
            delete=False,
        ) as temporary:
            temporary_name = temporary.name
            json.dump(
                payload,
                temporary,
                ensure_ascii=True,
                separators=(",", ":"),
                sort_keys=True,
            )
            temporary.write("\n")
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_name, destination)
        temporary_name = None
    finally:
        if temporary_name is not None:
            Path(temporary_name).unlink(missing_ok=True)
