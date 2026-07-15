from __future__ import annotations

import threading

from yap_server.pools.batch_asr import (
    BatchAsrJob,
    WorkerContainmentError,
    WorkerExecutionError,
)
from yap_server.pools.model_lock import LockedFixture, ModelPoolLock


IMAGE_ID = "sha256:" + "e" * 64
AUDIO_SHA256 = "f" * 64
CHECKED_HEAD = "a" * 40
STORAGE_NAMESPACE = "storage-test"


def test_lock() -> ModelPoolLock:
    return ModelPoolLock(
        schema_version=1,
        runtime_image="registry.example/asr",
        runtime_source="https://example.invalid/runtime",
        runtime_license="Example runtime license",
        runtime_platform="linux/arm64",
        runtime_digest="sha256:" + "a" * 64,
        runtime_source_tag="26.06-py3",
        runtime_python_version="3.12",
        runtime_torch_version="2.13.0a0+example",
        runtime_cuda_version="13.3.0",
        runtime_torch_cuda_version="13.3",
        runtime_overlay_packages=(("transformers", "5.13.1"),),
        pool_id="cohere-batch",
        model_id="CohereLabs/cohere-transcribe-03-2026",
        model_revision="b" * 40,
        model_license="Apache-2.0",
        model_source="https://example.invalid/upstream",
        model_distribution_id="example/cohere-distribution",
        model_distribution_revision="c" * 40,
        model_distribution_source="https://example.invalid/distribution",
        model_distribution_provenance="verified test distribution",
        supported_languages=("en",),
        artifacts=(),
        fixture=LockedFixture(
            path="fixture.wav",
            source="https://example.invalid/fixture.wav",
            license="CC-BY-4.0",
            sha256="d" * 64,
            golden_transcript="fixture",
        ),
    )


def valid_worker_result(lock: ModelPoolLock) -> dict[str, object]:
    return {
        "schemaVersion": 1,
        "jobId": "job-1",
        "model": {
            "poolId": lock.pool_id,
            "id": lock.model_id,
            "revision": lock.model_revision,
        },
        "audio": {
            "sha256": AUDIO_SHA256,
            "durationMs": 100,
            "sampleRateHz": 16000,
        },
        "transcript": {
            "text": "hello",
            "language": "en",
            "punctuation": True,
        },
        "runtime": {
            "device": "cuda",
            "pythonVersion": "3.12.9",
            "torchVersion": lock.runtime_torch_version,
            "torchCudaVersion": lock.runtime_torch_cuda_version,
            "overlayPackages": dict(lock.runtime_overlay_packages),
        },
    }


class BlockingWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.release = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        _cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        self.release.wait(timeout=5)
        return {"schemaVersion": 1, "jobId": job.job_id}


class ClosableWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.closed = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        _cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        self.closed.wait(timeout=0.25)
        return {"schemaVersion": 1, "jobId": job.job_id}

    def close(self) -> None:
        self.closed.set()


class CancellationAwareWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.stopped = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        if not cancellation.wait(timeout=5):
            raise AssertionError(f"job {job.job_id} was not cancelled")
        self.stopped.set()
        raise WorkerExecutionError("isolated ASR worker was cancelled")


class ContainmentFailureWorker:
    def run(
        self,
        job: BatchAsrJob,
        _cancellation: threading.Event,
    ) -> dict[str, object]:
        raise WorkerContainmentError(
            f"container cleanup could not be verified for {job.job_id}"
        )
