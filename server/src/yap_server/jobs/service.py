from __future__ import annotations

from copy import deepcopy
from datetime import datetime
import hashlib
from pathlib import Path
import threading
from typing import Callable, Mapping, Protocol, Sequence
from uuid import uuid4

from yap_server.pools.batch_contract import (
    BatchAsrJob,
    PoolBackpressure,
)
from .artifacts import (
    publish_wav as _publish_wav,
    sha256_file as _sha256_file,
)
from .chunk_contract import (
    chunk_path as _chunk_path,
    validated_single_track_chunks as _validated_single_track_chunks,
)
from .chunk_upload import ChunkUploadCoordinator, ChunkUploadPlan
from .completion import JobCompletionCoordinator
from .contract_values import (
    MAX_CLIENT_CLOCK_SKEW as _MAX_CLIENT_CLOCK_SKEW,
    TERMINAL_STATUSES as _TERMINAL_STATUSES,
    identifier as _identifier,
    mapping as _mapping,
    text as _text,
    utc_timestamp as _utc_timestamp,
)
from .errors import JobServiceError
from .intake_contract import (
    selected_language as _selected_language,
    validate_create_request as _validate_create_request,
)
from .job_store import DurableJobState, RecordingJobStore


_MAX_STORED_JOBS = 512
_CANCELLATION_ACK_TIMEOUT_SECONDS = 2.0


class BatchJobProcessor(Protocol):
    def submit(self, job: BatchAsrJob): ...

    def cancel(self, job_id: str) -> bool: ...


class RecordingJobService:
    """Owns immutable job intake and the server-side batch lifecycle."""

    def __init__(
        self,
        storage_root: Path,
        *,
        processor: BatchJobProcessor,
        supported_languages: Sequence[str],
        now: Callable[[], str],
        cancellation_timeout_seconds: float = _CANCELLATION_ACK_TIMEOUT_SECONDS,
        startup_worker_cleanup_verified: bool = False,
    ) -> None:
        if cancellation_timeout_seconds <= 0:
            raise ValueError("cancellation timeout must be positive")
        if not isinstance(startup_worker_cleanup_verified, bool):
            raise ValueError("startup cleanup verification must be boolean")
        self._processor = processor
        self._supported_languages = frozenset(supported_languages)
        self._now = now
        self._cancellation_timeout_seconds = cancellation_timeout_seconds
        self._store = RecordingJobStore(
            storage_root,
            supported_languages=supported_languages,
            now=now,
            startup_worker_cleanup_verified=startup_worker_cleanup_verified,
        )
        self._storage_root = self._store.root
        self._lock = threading.RLock()
        self._state: DurableJobState = self._store.load()
        self._committing: set[str] = set()
        self._futures: dict[str, object] = {}
        self._completion_events: dict[str, threading.Event] = {}
        self._uploads = ChunkUploadCoordinator(
            storage_root=self._storage_root,
            state=self._state,
            store=self._store,
            lock=self._lock,
            now=self._now,
        )
        self._completion = JobCompletionCoordinator(
            storage_root=self._storage_root,
            state=self._state,
            store=self._store,
            futures=self._futures,
            completion_events=self._completion_events,
            lock=self._lock,
            now=self._now,
        )
        with self._lock:
            self._prune_expired_jobs_locked(
                _utc_timestamp(self._now(), "server clock")
            )

    def create(
        self,
        request: Mapping[str, object],
        *,
        idempotency_key: str | None = None,
    ) -> dict[str, object]:
        try:
            _validate_create_request(request, self._supported_languages)
            if idempotency_key is not None:
                _identifier(idempotency_key, 128, "create idempotency key")
        except ValueError as error:
            raise JobServiceError(
                400,
                "INVALID_JOB",
                "Recording job declaration is invalid.",
            ) from error
        metadata = _mapping(request.get("metadata"), "metadata")
        capture_manifest = _mapping(
            request.get("captureManifest"),
            "captureManifest",
        )
        started_at = _utc_timestamp(metadata.get("startedAtUtc"), "startedAtUtc")
        retention_at = _utc_timestamp(
            metadata.get("retentionExpiresAtUtc"),
            "retentionExpiresAtUtc",
        )
        session_id = _text(metadata.get("sessionId"), "metadata.sessionId")
        if capture_manifest.get("sessionId") != session_id:
            raise ValueError("capture manifest session does not match metadata")
        display_name = _text(request.get("displayName"), "displayName")
        if request.get("route") != "server_batch":
            raise ValueError("route must be server_batch")
        with self._lock:
            created_at = self._now()
            server_now = _utc_timestamp(created_at, "server clock")
            if (
                retention_at <= server_now
                or started_at > server_now + _MAX_CLIENT_CLOCK_SKEW
            ):
                raise JobServiceError(
                    400,
                    "INVALID_JOB",
                    "Recording job retention or capture time is invalid.",
                )
            self._prune_expired_jobs_locked(
                server_now
            )
            if idempotency_key is not None:
                existing_job_id = self._state.created_by_key.get(idempotency_key)
                if existing_job_id is not None:
                    if self._state.requests[existing_job_id] != dict(request):
                        raise JobServiceError(
                            409,
                            "CREATE_IDEMPOTENCY_CONFLICT",
                            "The create idempotency key is already bound to different content.",
                        )
                    return deepcopy(self._state.jobs[existing_job_id])
            if len(self._state.jobs) >= _MAX_STORED_JOBS:
                raise JobServiceError(
                    429,
                    "SERVER_STORAGE_LIMIT",
                    "Private recording storage reached its configured job limit.",
                )
            job_id = f"job-{uuid4().hex}"
            projection: dict[str, object] = {
                "jobId": job_id,
                "sessionId": session_id,
                "displayName": display_name,
                "sessionMode": _text(metadata.get("mode"), "metadata.mode"),
                "sessionOrigin": _text(metadata.get("origin"), "metadata.origin"),
                "status": "accepted",
                "route": "server_batch",
                "captureManifest": deepcopy(capture_manifest),
                "createdAtUtc": created_at,
                "updatedAtUtc": created_at,
            }
            job_root = self._storage_root / "jobs" / job_id
            (job_root / "chunks").mkdir(parents=True, exist_ok=False)
            self._state.jobs[job_id] = projection
            self._state.requests[job_id] = deepcopy(dict(request))
            self._state.create_keys[job_id] = idempotency_key
            if idempotency_key is not None:
                self._state.created_by_key[idempotency_key] = job_id
            try:
                self._persist_job_locked(job_id)
            except Exception:
                self._delete_job_locked(job_id)
                raise
        return deepcopy(projection)

    def get(self, job_id: str) -> dict[str, object]:
        with self._lock:
            return deepcopy(self._state.jobs[job_id])

    def prepare_chunk_upload(
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
        return self._uploads.prepare(
            job_id,
            track_id=track_id,
            sequence_start=sequence_start,
            sequence_end=sequence_end,
            idempotency_key=idempotency_key,
            content_sha256=content_sha256,
            audio_codec=audio_codec,
            sample_rate_hz=sample_rate_hz,
            channels=channels,
            content_length=content_length,
        )


    def accept_chunk(
        self,
        plan: ChunkUploadPlan,
        body: bytes,
    ) -> dict[str, object]:
        return self._uploads.accept(plan, body)


    def commit(
        self,
        job_id: str,
        request: Mapping[str, object],
    ) -> dict[str, object]:
        with self._lock:
            creation = self._state.requests[job_id]
            job = self._state.jobs[job_id]
            if (
                job["status"] not in {"accepted", "uploading"}
                or job_id in self._committing
            ):
                raise JobServiceError(
                    409,
                    "JOB_NOT_COMMITTABLE",
                    "The recording job cannot be committed from its current state.",
                )
            if request.get("captureManifest") != creation.get("captureManifest"):
                raise ValueError("commit manifest does not match job creation")
            chunks = creation.get("chunks")
            if not isinstance(chunks, list) or request.get("chunkCount") != len(chunks):
                raise ValueError("commit chunk count does not match job creation")
            ordered_chunks = _validated_single_track_chunks(chunks)
            job_root = self._storage_root / "jobs" / job_id
            chunk_paths = [
                _chunk_path(job_root, _mapping(chunk.get("replayKey"), "replayKey"))
                for chunk in ordered_chunks
            ]
            for chunk, path in zip(ordered_chunks, chunk_paths, strict=True):
                content = _mapping(chunk.get("contentIdentity"), "contentIdentity")
                if not path.is_file():
                    raise ValueError("every declared chunk must be uploaded before commit")
                body = path.read_bytes()
                if len(body) != content.get("byteLength") or (
                    hashlib.sha256(body).hexdigest() != content.get("sha256")
                ):
                    raise ValueError("an uploaded chunk no longer matches its identity")
            language_bcp47 = _selected_language(creation, self._supported_languages)
            self._committing.add(job_id)

        future: object | None = None
        completion_event: threading.Event | None = None
        commit_error: BaseException | None = None
        try:
            input_path = job_root / "input.wav"
            _publish_wav(input_path, chunk_paths)
            input_sha256 = _sha256_file(input_path)
            worker_job = BatchAsrJob(
                job_id=job_id,
                input_path=input_path,
                result_path=job_root / "worker-result.json",
                language=language_bcp47.split("-", 1)[0].lower(),
                input_sha256=input_sha256,
            )
            with self._lock:
                job = self._state.jobs[job_id]
                if job_id in self._state.cancelled:
                    self._finalize_cancellation_locked(job_id)
                    return deepcopy(self._state.jobs[job_id])
                previous_status = job["status"]
                previous_updated_at = job["updatedAtUtc"]
                committed_at = self._now()
                job["status"] = "server_processing"
                job["updatedAtUtc"] = committed_at
                try:
                    self._persist_job_locked(job_id)
                except BaseException:
                    job["status"] = previous_status
                    job["updatedAtUtc"] = previous_updated_at
                    raise
                try:
                    future = self._processor.submit(worker_job)
                except PoolBackpressure as error:
                    job["status"] = previous_status
                    job["updatedAtUtc"] = previous_updated_at
                    self._persist_job_locked(job_id)
                    raise JobServiceError(
                        429,
                        "SERVER_BUSY",
                        "Server capacity is temporarily unavailable.",
                        retryable=True,
                    ) from error
                except BaseException:
                    job["status"] = previous_status
                    job["updatedAtUtc"] = previous_updated_at
                    self._persist_job_locked(job_id)
                    raise
                self._futures[job_id] = future
                completion_event = threading.Event()
                self._completion_events[job_id] = completion_event
                projection = deepcopy(job)
        except BaseException as error:
            commit_error = error
            raise
        finally:
            with self._lock:
                self._committing.discard(job_id)
                if job_id in self._state.cancelled and job_id not in self._futures:
                    try:
                        self._finalize_cancellation_locked(job_id)
                    except Exception:
                        if commit_error is None:
                            raise
            if future is not None:
                assert completion_event is not None
                future.add_done_callback(
                    lambda completed: self._completion.finish_safely(
                        job_id,
                        language_bcp47,
                        completed,
                        completion_event,
                    )
                )
        assert future is not None
        return projection

    def cancel(self, job_id: str) -> dict[str, object]:
        future: object | None = None
        completion_event: threading.Event | None = None
        with self._lock:
            job = self._state.jobs[job_id]
            error = job.get("error")
            if (
                job.get("status") == "failed"
                and isinstance(error, Mapping)
                and error.get("code") == "ASR_CLEANUP_UNVERIFIED"
            ):
                raise JobServiceError(
                    503,
                    "CANCELLATION_CLEANUP_UNVERIFIED",
                    "Worker cleanup could not be verified.",
                    retryable=True,
                )
            status = job["status"]
            if status == "cancelled":
                if job_id in self._committing or job_id in self._futures:
                    self._persist_job_locked(job_id)
                else:
                    self._purge_private_audio_locked(job_id)
                return deepcopy(job)
            future = self._futures.get(job_id)
            self._state.cancelled.add(job_id)
            if job_id in self._committing or future is not None:
                try:
                    self._persist_job_locked(job_id)
                except BaseException:
                    self._state.cancelled.discard(job_id)
                    raise
                completion_event = self._completion_events.get(job_id)
                if completion_event is None and job_id in self._committing:
                    completion_event = threading.Event()
                    self._completion_events[job_id] = completion_event
            else:
                self._finalize_cancellation_locked(job_id)
                return deepcopy(self._state.jobs[job_id])
        if future is not None:
            cancel_processor = getattr(self._processor, "cancel", None)
            if callable(cancel_processor):
                cancel_processor(job_id)
            else:
                future.cancel()
        if completion_event is None or not completion_event.wait(
            timeout=self._cancellation_timeout_seconds
        ):
            raise JobServiceError(
                503,
                "CANCELLATION_PENDING",
                "Worker cleanup is still pending.",
                retryable=True,
            )
        with self._lock:
            job = self._state.jobs[job_id]
            error = job.get("error")
            if (
                job.get("status") == "failed"
                and isinstance(error, Mapping)
                and error.get("code") == "ASR_CLEANUP_UNVERIFIED"
            ):
                raise JobServiceError(
                    503,
                    "CANCELLATION_CLEANUP_UNVERIFIED",
                    "Worker cleanup could not be verified.",
                    retryable=True,
                )
            self._finalize_cancellation_locked(job_id)
            return deepcopy(self._state.jobs[job_id])

    def _finalize_cancellation_locked(self, job_id: str) -> None:
        job = self._state.jobs[job_id]
        job["status"] = "cancelled"
        job["updatedAtUtc"] = self._now()
        job.pop("error", None)
        self._purge_private_audio_locked(job_id)
        completion_event = self._completion_events.pop(job_id, None)
        if completion_event is not None:
            completion_event.set()

    def get_result(self, job_id: str) -> dict[str, object]:
        with self._lock:
            if job_id not in self._state.jobs:
                raise JobServiceError(
                    404,
                    "JOB_NOT_FOUND",
                    "Recording job not found.",
                )
            if job_id not in self._state.results:
                raise JobServiceError(
                    409,
                    "RESULT_NOT_READY",
                    "The immutable transcript result is not available yet.",
                    retryable=self._state.jobs[job_id].get("status") != "failed",
                )
            return deepcopy(self._state.results[job_id])

    def _purge_private_audio_locked(self, job_id: str) -> None:
        self._store.purge_private_audio(self._state, job_id)

    def prune_expired(self) -> int:
        with self._lock:
            return self._prune_expired_jobs_locked(
                _utc_timestamp(self._now(), "server clock")
            )

    def _prune_expired_jobs_locked(self, now: datetime) -> int:
        expired: list[str] = []
        for job_id, job in self._state.jobs.items():
            metadata = _mapping(self._state.requests[job_id].get("metadata"), "metadata")
            retention = metadata.get("retentionExpiresAtUtc")
            if retention is not None and _utc_timestamp(
                retention,
                "retentionExpiresAtUtc",
            ) <= now:
                expired.append(job_id)
        deleted = 0
        for job_id in expired:
            job = self._state.jobs[job_id]
            if job.get("status") not in _TERMINAL_STATUSES:
                if job_id not in self._state.cancelled:
                    self._state.cancelled.add(job_id)
                    self._persist_job_locked(job_id)
                future = self._futures.get(job_id)
                if future is not None:
                    cancel_processor = getattr(self._processor, "cancel", None)
                    if callable(cancel_processor):
                        cancel_processor(job_id)
                    else:
                        future.cancel()
                if job_id in self._committing or job_id in self._futures:
                    continue
                self._finalize_cancellation_locked(job_id)
            if job.get("status") == "cancelled":
                self._purge_private_audio_locked(job_id)
            self._delete_job_locked(job_id)
            deleted += 1
        return deleted

    def _delete_job_locked(self, job_id: str) -> None:
        self._store.delete(self._state, job_id)

    def _persist_job_locked(self, job_id: str) -> None:
        self._store.persist(self._state, job_id)
