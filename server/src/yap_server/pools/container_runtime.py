from __future__ import annotations

import re
import subprocess
import threading
import time
from typing import BinaryIO, Callable

from yap_server.pools.batch_contract import (
    WorkerContainmentError,
    WorkerExecutionError,
)


MAX_WORKER_OUTPUT_BYTES = 1024 * 1024
_PROCESS_READ_BYTES = 64 * 1024
_CONTAINER_CLEANUP_TIMEOUT_SECONDS = 30
_CONTAINER_ID = re.compile(r"^[0-9a-f]{12,64}$")
CONTAINER_LABEL_VALUE = re.compile(r"^[A-Za-z0-9._-]{1,64}$")
OWNER_LABEL = "com.mcnatg1.yap.owner"
OWNER_VALUE = "batch-asr"
STORAGE_LABEL = "com.mcnatg1.yap.storage"
RUNTIME_LABEL = "com.mcnatg1.yap.runtime"
JOB_LABEL = "com.mcnatg1.yap.job"
REVISION_LABEL = "org.opencontainers.image.revision"


def force_remove_container(
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
    if CONTAINER_LABEL_VALUE.fullmatch(storage_namespace) is None:
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
                f"label={OWNER_LABEL}={OWNER_VALUE}",
                "--filter",
                f"label={STORAGE_LABEL}={storage_namespace}",
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
        force_remove_container(docker_binary, container_id, runner=runner)
    return len(container_ids)


def validate_worker_output(completed: subprocess.CompletedProcess[str]) -> None:
    for stream_name, value in (
        ("stdout", completed.stdout),
        ("stderr", completed.stderr),
    ):
        if not isinstance(value, str):
            raise WorkerExecutionError(
                f"isolated ASR worker {stream_name} was not decoded text"
            )
        if len(value.encode("utf-8", errors="replace")) > MAX_WORKER_OUTPUT_BYTES:
            raise WorkerExecutionError(
                f"isolated ASR worker {stream_name} exceeded the bounded output"
            )


def run_bounded_process(
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
