from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
import threading
from typing import Protocol


_JOB_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
_LANGUAGE = re.compile(r"^[a-z]{2}$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")


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
