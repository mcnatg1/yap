from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile

from yap_server.pools.batch_contract import BatchAsrJob, WorkerExecutionError
from yap_server.pools.model_lock import ModelPoolLock


def validate_result(
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


def publish_result(path: Path, payload: dict[str, object]) -> None:
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
