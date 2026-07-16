from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass
import hashlib
import os
from pathlib import Path
import tempfile
import threading
from typing import Callable

from .chunk_contract import find_chunk, receipt_key
from .contract_values import mapping
from .errors import JobServiceError
from .job_store import DurableJobState, RecordingJobStore


@dataclass(frozen=True, slots=True)
class ChunkUploadPlan:
    job_id: str
    replay_key: dict[str, object]
    content_identity: dict[str, object]
    destination: Path
    receipt_key: tuple[object, ...]


class ChunkUploadCoordinator:
    """Admits declared PCM chunks and persists their replay-safe receipts."""

    def __init__(
        self,
        *,
        storage_root: Path,
        state: DurableJobState,
        store: RecordingJobStore,
        lock: threading.RLock,
        now: Callable[[], str],
    ) -> None:
        self._storage_root = storage_root
        self._state = state
        self._store = store
        self._lock = lock
        self._now = now

    def prepare(
        self,
        job_id: str,
        *,
        track_id: str,
        sequence_start: int,
        sequence_end: int,
        idempotency_key: str,
        content_sha256: str,
        audio_codec: str,
        sample_rate_hz: int,
        channels: int,
        content_length: int,
    ) -> ChunkUploadPlan:
        with self._lock:
            status = self._state.jobs[job_id]["status"]
            if status not in {"accepted", "uploading"}:
                raise JobServiceError(
                    409,
                    "JOB_NOT_UPLOADABLE",
                    "The recording job no longer accepts chunks.",
                )
            request = self._state.requests[job_id]
            expected = find_chunk(
                request,
                track_id=track_id,
                sequence_start=sequence_start,
                sequence_end=sequence_end,
            )
            replay_key = mapping(expected.get("replayKey"), "replayKey")
            content_identity = mapping(expected.get("contentIdentity"), "contentIdentity")
            expected_key = (
                f"{replay_key['schemaVersion']}/{replay_key['sessionId']}/"
                f"{replay_key['trackId']}/{replay_key['sequenceStart']}/"
                f"{replay_key['sequenceEnd']}"
            )
            if idempotency_key == expected_key and (
                content_sha256 != content_identity.get("sha256")
            ):
                raise JobServiceError(
                    409,
                    "CONTENT_IDENTITY_CONFLICT",
                    "The chunk replay key is already bound to different content.",
                )
            declared = (
                idempotency_key == expected_key
                and content_sha256 == content_identity.get("sha256")
                and audio_codec == expected.get("audioCodec")
                and sample_rate_hz == expected.get("sampleRateHz")
                and channels == expected.get("channels")
                and content_length == content_identity.get("byteLength")
            )
            if not declared:
                raise ValueError("chunk path or headers do not match the immutable declaration")
            filename = (
                f"{replay_key['trackId']}-{replay_key['sequenceStart']}-"
                f"{replay_key['sequenceEnd']}.pcm"
            )
            return ChunkUploadPlan(
                job_id=job_id,
                replay_key=deepcopy(dict(replay_key)),
                content_identity=deepcopy(dict(content_identity)),
                destination=self._storage_root / "jobs" / job_id / "chunks" / filename,
                receipt_key=receipt_key(job_id, replay_key),
            )

    def accept(self, plan: ChunkUploadPlan, body: bytes) -> dict[str, object]:
        expected_length = plan.content_identity["byteLength"]
        expected_sha256 = plan.content_identity["sha256"]
        if len(body) != expected_length:
            raise ValueError("chunk body length does not match its declared identity")
        if hashlib.sha256(body).hexdigest() != expected_sha256:
            raise ValueError("chunk body hash does not match its declared identity")

        with self._lock:
            status = self._state.jobs[plan.job_id]["status"]
            if status not in {"accepted", "uploading"}:
                raise JobServiceError(
                    409,
                    "JOB_NOT_UPLOADABLE",
                    "The recording job no longer accepts chunks.",
                )
            existing = self._state.receipts.get(plan.receipt_key)
            if existing is not None:
                self._store.persist(self._state, plan.job_id)
                replay = deepcopy(existing)
                replay["disposition"] = "replayed"
                return replay

            temporary_path: Path | None = None
            try:
                with tempfile.NamedTemporaryFile(
                    mode="wb",
                    dir=plan.destination.parent,
                    prefix=".upload-",
                    delete=False,
                ) as temporary:
                    temporary_path = Path(temporary.name)
                    temporary.write(body)
                    temporary.flush()
                    os.fsync(temporary.fileno())
                os.replace(temporary_path, plan.destination)
                temporary_path = None
            finally:
                if temporary_path is not None:
                    temporary_path.unlink(missing_ok=True)

            accepted_at = self._now()
            job = self._state.jobs[plan.job_id]
            job["status"] = "uploading"
            job["updatedAtUtc"] = accepted_at
            receipt = {
                "replayKey": deepcopy(plan.replay_key),
                "contentIdentity": deepcopy(plan.content_identity),
                "disposition": "accepted",
                "acceptedAtUtc": accepted_at,
            }
            self._state.receipts[plan.receipt_key] = receipt
            self._store.persist(self._state, plan.job_id)
            return deepcopy(receipt)
