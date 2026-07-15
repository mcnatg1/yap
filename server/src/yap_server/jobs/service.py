from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import tempfile
import threading
from typing import Callable, Mapping, Protocol, Sequence
from uuid import uuid4
import wave

from yap_server.pools.batch_asr import (
    BatchAsrJob,
    PoolBackpressure,
    WorkerContainmentError,
)


_OPAQUE_ID = re.compile(r"^[A-Za-z0-9_-]+$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_BCP47 = re.compile(r"^[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*$")
_COUNTRY = re.compile(r"^[A-Z]{2}$")
_MAX_CHUNKS = 4096
_MAX_TRACKS = 8
_MAX_CHUNK_BYTES = 1024 * 1024
_MAX_JOB_PCM_BYTES = 16000 * 2 * 4 * 60 * 60
_MAX_STATE_BYTES = 2 * 1024 * 1024
_MAX_TRANSCRIPT_BYTES = 1024 * 1024
_MAX_MODEL_PROVENANCE_CHARS = 256
_MAX_STORED_JOBS = 512
_MAX_PRIVATE_RETENTION = timedelta(days=30)
_MAX_CLIENT_CLOCK_SKEW = timedelta(minutes=5)
_CANCELLATION_ACK_TIMEOUT_SECONDS = 2.0
_JOB_DIRECTORY = re.compile(r"^job-[0-9a-f]{32}$")
_TERMINAL_STATUSES = frozenset({"complete", "partial", "failed", "cancelled"})
_JOB_STATUSES = frozenset(
    {
        "accepted",
        "uploading",
        "server_processing",
        *_TERMINAL_STATUSES,
    }
)
_PERSISTED_ERROR_CODES = frozenset(
    {
        "ASR_RESULT_INVALID",
        "ASR_RESULT_PUBLISH_FAILED",
        "ASR_CLEANUP_UNVERIFIED",
        "ASR_WORKER_FAILED",
        "SERVER_RESTARTED",
        "SERVER_STORAGE_ERROR",
    }
)


class BatchJobProcessor(Protocol):
    def submit(self, job: BatchAsrJob): ...

    def cancel(self, job_id: str) -> bool: ...


class JobServiceError(RuntimeError):
    def __init__(
        self,
        status: int,
        code: str,
        message: str,
        *,
        retryable: bool = False,
    ) -> None:
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message
        self.retryable = retryable


@dataclass(frozen=True, slots=True)
class ChunkUploadPlan:
    job_id: str
    replay_key: dict[str, object]
    content_identity: dict[str, object]
    destination: Path
    receipt_key: tuple[object, ...]


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
        self._storage_root = storage_root.resolve()
        self._storage_root.mkdir(parents=True, exist_ok=True)
        self._processor = processor
        self._supported_languages = frozenset(supported_languages)
        self._now = now
        self._cancellation_timeout_seconds = cancellation_timeout_seconds
        self._startup_worker_cleanup_verified = startup_worker_cleanup_verified
        self._lock = threading.RLock()
        self._jobs: dict[str, dict[str, object]] = {}
        self._requests: dict[str, dict[str, object]] = {}
        self._results: dict[str, dict[str, object]] = {}
        self._receipts: dict[tuple[object, ...], dict[str, object]] = {}
        self._cancelled: set[str] = set()
        self._create_keys: dict[str, str | None] = {}
        self._created_by_key: dict[str, str] = {}
        self._committing: set[str] = set()
        self._futures: dict[str, object] = {}
        self._completion_events: dict[str, threading.Event] = {}
        self._load_existing_jobs()
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
                existing_job_id = self._created_by_key.get(idempotency_key)
                if existing_job_id is not None:
                    if self._requests[existing_job_id] != dict(request):
                        raise JobServiceError(
                            409,
                            "CREATE_IDEMPOTENCY_CONFLICT",
                            "The create idempotency key is already bound to different content.",
                        )
                    return deepcopy(self._jobs[existing_job_id])
            if len(self._jobs) >= _MAX_STORED_JOBS:
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
            self._jobs[job_id] = projection
            self._requests[job_id] = deepcopy(dict(request))
            self._create_keys[job_id] = idempotency_key
            if idempotency_key is not None:
                self._created_by_key[idempotency_key] = job_id
            try:
                self._persist_job_locked(job_id)
            except Exception:
                self._delete_job_locked(job_id)
                raise
        return deepcopy(projection)

    def get(self, job_id: str) -> dict[str, object]:
        with self._lock:
            return deepcopy(self._jobs[job_id])

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
        with self._lock:
            status = self._jobs[job_id]["status"]
            if status not in {"accepted", "uploading"}:
                raise JobServiceError(
                    409,
                    "JOB_NOT_UPLOADABLE",
                    "The recording job no longer accepts chunks.",
                )
            request = self._requests[job_id]
            expected = _find_chunk(
                request,
                track_id=track_id,
                sequence_start=sequence_start,
                sequence_end=sequence_end,
            )
            replay_key = _mapping(expected.get("replayKey"), "replayKey")
            content_identity = _mapping(
                expected.get("contentIdentity"),
                "contentIdentity",
            )
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
                receipt_key=(
                    job_id,
                    replay_key["schemaVersion"],
                    replay_key["sessionId"],
                    replay_key["trackId"],
                    replay_key["sequenceStart"],
                    replay_key["sequenceEnd"],
                ),
            )

    def accept_chunk(
        self,
        plan: ChunkUploadPlan,
        body: bytes,
    ) -> dict[str, object]:
        expected_length = plan.content_identity["byteLength"]
        expected_sha256 = plan.content_identity["sha256"]
        if len(body) != expected_length:
            raise ValueError("chunk body length does not match its declared identity")
        if hashlib.sha256(body).hexdigest() != expected_sha256:
            raise ValueError("chunk body hash does not match its declared identity")

        with self._lock:
            status = self._jobs[plan.job_id]["status"]
            if status not in {"accepted", "uploading"}:
                raise JobServiceError(
                    409,
                    "JOB_NOT_UPLOADABLE",
                    "The recording job no longer accepts chunks.",
                )
            existing = self._receipts.get(plan.receipt_key)
            if existing is not None:
                self._persist_job_locked(plan.job_id)
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
            job = self._jobs[plan.job_id]
            job["status"] = "uploading"
            job["updatedAtUtc"] = accepted_at
            receipt = {
                "replayKey": deepcopy(plan.replay_key),
                "contentIdentity": deepcopy(plan.content_identity),
                "disposition": "accepted",
                "acceptedAtUtc": accepted_at,
            }
            self._receipts[plan.receipt_key] = receipt
            self._persist_job_locked(plan.job_id)
            return deepcopy(receipt)

    def commit(
        self,
        job_id: str,
        request: Mapping[str, object],
    ) -> dict[str, object]:
        with self._lock:
            creation = self._requests[job_id]
            job = self._jobs[job_id]
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
                job = self._jobs[job_id]
                if job_id in self._cancelled:
                    self._finalize_cancellation_locked(job_id)
                    return deepcopy(self._jobs[job_id])
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
                if job_id in self._cancelled and job_id not in self._futures:
                    try:
                        self._finalize_cancellation_locked(job_id)
                    except Exception:
                        if commit_error is None:
                            raise
            if future is not None:
                assert completion_event is not None
                future.add_done_callback(
                    lambda completed: self._finish_job_safely(
                        job_id,
                        language_bcp47,
                        completed,
                        completion_event,
                    )
                )
        assert future is not None
        return projection

    def _finish_job_safely(
        self,
        job_id: str,
        language_bcp47: str,
        future: object,
        completion_event: threading.Event,
    ) -> None:
        try:
            self._finish_job(job_id, language_bcp47, future)
        except Exception:
            # Future callbacks are an outer trust boundary. Never let a storage
            # exception reach concurrent.futures' default callback logger,
            # which would print filesystem details. Preserve an already
            # published complete result for restart reconciliation; otherwise
            # converge to the existing generic retryable failure tombstone.
            try:
                with self._lock:
                    self._discard_future_locked(job_id, future)
                    job = self._jobs.get(job_id)
                    if job is None or job.get("status") in {"complete", "partial"}:
                        return
                    if job_id not in self._cancelled and job.get("status") != "failed":
                        job["status"] = "failed"
                        job["updatedAtUtc"] = self._now()
                        job["error"] = {
                            "code": "SERVER_STORAGE_ERROR",
                            "message": "Private result storage did not complete safely.",
                            "retryable": True,
                            "requestId": f"job-{job_id}",
                        }
                    try:
                        self._purge_private_audio_locked(job_id)
                    except Exception:
                        pass
            except Exception:
                pass
        finally:
            completion_event.set()
            with self._lock:
                if self._completion_events.get(job_id) is completion_event:
                    self._completion_events.pop(job_id, None)

    def cancel(self, job_id: str) -> dict[str, object]:
        future: object | None = None
        completion_event: threading.Event | None = None
        with self._lock:
            job = self._jobs[job_id]
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
            self._cancelled.add(job_id)
            if job_id in self._committing or future is not None:
                try:
                    self._persist_job_locked(job_id)
                except BaseException:
                    self._cancelled.discard(job_id)
                    raise
                completion_event = self._completion_events.get(job_id)
                if completion_event is None and job_id in self._committing:
                    completion_event = threading.Event()
                    self._completion_events[job_id] = completion_event
            else:
                self._finalize_cancellation_locked(job_id)
                return deepcopy(self._jobs[job_id])
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
            job = self._jobs[job_id]
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
            return deepcopy(self._jobs[job_id])

    def _finalize_cancellation_locked(self, job_id: str) -> None:
        job = self._jobs[job_id]
        job["status"] = "cancelled"
        job["updatedAtUtc"] = self._now()
        job.pop("error", None)
        self._purge_private_audio_locked(job_id)
        completion_event = self._completion_events.pop(job_id, None)
        if completion_event is not None:
            completion_event.set()

    def get_result(self, job_id: str) -> dict[str, object]:
        with self._lock:
            if job_id not in self._jobs:
                raise JobServiceError(
                    404,
                    "JOB_NOT_FOUND",
                    "Recording job not found.",
                )
            if job_id not in self._results:
                raise JobServiceError(
                    409,
                    "RESULT_NOT_READY",
                    "The immutable transcript result is not available yet.",
                    retryable=self._jobs[job_id].get("status") != "failed",
                )
            return deepcopy(self._results[job_id])

    def _finish_job(self, job_id: str, language_bcp47: str, future: object) -> None:
        try:
            payload = future.result()
        except WorkerContainmentError:
            self._mark_containment_unverified(job_id, future)
            return
        except Exception:
            self._mark_failed_unless_cancelled(
                job_id,
                future,
                code="ASR_WORKER_FAILED",
                message="The private ASR worker did not complete the job.",
            )
            return
        try:
            worker_payload = _mapping(payload, "worker result")
            transcript = _mapping(
                worker_payload.get("transcript"),
                "worker transcript",
            )
            model = _mapping(worker_payload.get("model"), "worker model")
            transcript_text = _text(transcript.get("text"), "worker transcript.text")
            if (
                not transcript_text.strip()
                or len(transcript_text.encode("utf-8")) > _MAX_TRANSCRIPT_BYTES
            ):
                raise ValueError("worker transcript is empty or oversized")
            model_id = _text(model.get("id"), "worker model.id")
            model_revision = _text(model.get("revision"), "worker model.revision")
            if (
                len(model_id) > _MAX_MODEL_PROVENANCE_CHARS
                or len(model_revision) > _MAX_MODEL_PROVENANCE_CHARS
            ):
                raise ValueError("worker model identity is oversized")
            created_at = self._now()
            result: dict[str, object]
            with self._lock:
                if job_id in self._cancelled:
                    self._discard_future_locked(job_id, future)
                    self._purge_private_audio_locked(job_id)
                    return
                job = self._jobs[job_id]
                capture_manifest = _mapping(
                    job["captureManifest"],
                    "captureManifest",
                )
                result = {
                    "sessionId": job["sessionId"],
                    "revision": 1,
                    "authority": "server_authoritative",
                    "createdAtUtc": created_at,
                    "captureManifestSha256": capture_manifest["sha256"],
                    "previousResultSha256": None,
                    "status": "complete",
                    "language": {
                        "languageBcp47": language_bcp47,
                        "confidence": None,
                    },
                    "transcript": transcript_text,
                    "alignedWords": [],
                    "modelProvenance": [
                        {
                            "modelId": model_id,
                            "revision": model_revision,
                            "calibrationRevision": "asr-not-applicable",
                        }
                    ],
                }
                _validate_result_revision(result, job)
        except (KeyError, TypeError, ValueError):
            self._mark_failed_unless_cancelled(
                job_id,
                future,
                code="ASR_RESULT_INVALID",
                message="The private ASR worker returned an invalid result.",
            )
            return
        result_path = self._storage_root / "jobs" / job_id / "result-revision.json"
        try:
            _publish_json(result_path, result)
        except OSError:
            self._mark_failed_unless_cancelled(
                job_id,
                future,
                code="ASR_RESULT_PUBLISH_FAILED",
                message="The private ASR result could not be stored safely.",
            )
            return
        with self._lock:
            if job_id in self._cancelled:
                self._discard_future_locked(job_id, future)
                self._purge_private_audio_locked(job_id)
                return
            self._results[job_id] = result
            job = self._jobs[job_id]
            job["status"] = "complete"
            job["updatedAtUtc"] = created_at
            self._discard_future_locked(job_id, future)
            self._persist_job_locked(job_id)

    def _mark_containment_unverified(self, job_id: str, future: object) -> None:
        failed_at = self._now()
        with self._lock:
            self._discard_future_locked(job_id, future)
            job = self._jobs[job_id]
            job["status"] = "failed"
            job["updatedAtUtc"] = failed_at
            job["error"] = {
                "code": "ASR_CLEANUP_UNVERIFIED",
                "message": "The private ASR worker cleanup could not be verified.",
                "retryable": True,
                "requestId": f"job-{job_id}",
            }
            self._purge_private_audio_locked(job_id)

    def _mark_failed_unless_cancelled(
        self,
        job_id: str,
        future: object,
        *,
        code: str,
        message: str,
    ) -> None:
        failed_at = self._now()
        with self._lock:
            self._discard_future_locked(job_id, future)
            if job_id in self._cancelled:
                self._purge_private_audio_locked(job_id)
                return
            job = self._jobs[job_id]
            job["status"] = "failed"
            job["updatedAtUtc"] = failed_at
            job["error"] = {
                "code": code,
                "message": message,
                "retryable": True,
                "requestId": f"job-{job_id}",
            }
            self._purge_private_audio_locked(job_id)

    def _discard_future_locked(self, job_id: str, future: object) -> None:
        if self._futures.get(job_id) is future:
            self._futures.pop(job_id, None)

    def _purge_private_audio_locked(self, job_id: str) -> None:
        job_root = self._storage_root / "jobs" / job_id
        chunks_root = job_root / "chunks"
        chunk_metadata = chunks_root.lstat()
        if stat.S_ISLNK(chunk_metadata.st_mode) or not stat.S_ISDIR(
            chunk_metadata.st_mode
        ):
            raise ValueError("private chunk storage is unsafe")
        for receipt_key in tuple(self._receipts):
            if receipt_key[0] == job_id:
                self._receipts.pop(receipt_key, None)
        self._results.pop(job_id, None)
        self._persist_job_locked(job_id)
        for entry in chunks_root.iterdir():
            _unlink_private_regular_file(entry, "private recording chunk")
        for name in (
            "input.wav",
            "input.wav.part",
            "worker-result.json",
            "result-revision.json",
        ):
            _unlink_private_regular_file(job_root / name, "private recording artifact")

    def prune_expired(self) -> int:
        with self._lock:
            return self._prune_expired_jobs_locked(
                _utc_timestamp(self._now(), "server clock")
            )

    def _prune_expired_jobs_locked(self, now: datetime) -> int:
        expired: list[str] = []
        for job_id, job in self._jobs.items():
            metadata = _mapping(self._requests[job_id].get("metadata"), "metadata")
            retention = metadata.get("retentionExpiresAtUtc")
            if retention is not None and _utc_timestamp(
                retention,
                "retentionExpiresAtUtc",
            ) <= now:
                expired.append(job_id)
        deleted = 0
        for job_id in expired:
            job = self._jobs[job_id]
            if job.get("status") not in _TERMINAL_STATUSES:
                if job_id not in self._cancelled:
                    self._cancelled.add(job_id)
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
        job_root = self._storage_root / "jobs" / job_id
        metadata = job_root.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise ValueError("expired job storage is unsafe")
        shutil.rmtree(job_root)
        self._jobs.pop(job_id, None)
        self._requests.pop(job_id, None)
        self._results.pop(job_id, None)
        self._cancelled.discard(job_id)
        create_key = self._create_keys.pop(job_id, None)
        if create_key is not None and self._created_by_key.get(create_key) == job_id:
            self._created_by_key.pop(create_key, None)
        for receipt_key in tuple(self._receipts):
            if receipt_key[0] == job_id:
                self._receipts.pop(receipt_key, None)

    def _persist_job_locked(self, job_id: str) -> None:
        receipts = [
            deepcopy(receipt)
            for key, receipt in self._receipts.items()
            if key[0] == job_id
        ]
        receipts.sort(
            key=lambda receipt: (
                receipt["replayKey"]["trackId"],
                receipt["replayKey"]["sequenceStart"],
                receipt["replayKey"]["sequenceEnd"],
            )
        )
        _publish_json(
            self._storage_root / "jobs" / job_id / "state.json",
            {
                "schemaVersion": 3,
                "createIdempotencyKey": self._create_keys[job_id],
                "cancellationRequested": job_id in self._cancelled,
                "creation": self._requests[job_id],
                "projection": self._jobs[job_id],
                "receipts": receipts,
            },
        )

    def _load_existing_jobs(self) -> None:
        jobs_root = self._storage_root / "jobs"
        if not jobs_root.exists():
            return
        if jobs_root.is_symlink() or not jobs_root.is_dir():
            raise ValueError("job storage root must be a real directory")
        for job_root in sorted(jobs_root.iterdir(), key=lambda path: path.name):
            if job_root.is_symlink() or not job_root.is_dir():
                raise ValueError("job storage contains an unsafe entry")
            job_id = job_root.name
            if _JOB_DIRECTORY.fullmatch(job_id) is None:
                raise ValueError("job storage contains an invalid job directory")
            state = _read_json_file(job_root / "state.json")
            schema_version = state.get("schemaVersion")
            if schema_version == 1:
                _exact_keys(
                    state,
                    {"schemaVersion", "creation", "projection", "receipts"},
                    "persisted job state",
                )
                create_idempotency_key = None
                cancellation_requested = False
            elif schema_version == 2:
                _exact_keys(
                    state,
                    {
                        "schemaVersion",
                        "createIdempotencyKey",
                        "creation",
                        "projection",
                        "receipts",
                    },
                    "persisted job state",
                )
                raw_create_key = state.get("createIdempotencyKey")
                create_idempotency_key = (
                    None
                    if raw_create_key is None
                    else _identifier(raw_create_key, 128, "create idempotency key")
                )
                cancellation_requested = False
            elif schema_version == 3:
                _exact_keys(
                    state,
                    {
                        "schemaVersion",
                        "createIdempotencyKey",
                        "cancellationRequested",
                        "creation",
                        "projection",
                        "receipts",
                    },
                    "persisted job state",
                )
                raw_create_key = state.get("createIdempotencyKey")
                create_idempotency_key = (
                    None
                    if raw_create_key is None
                    else _identifier(raw_create_key, 128, "create idempotency key")
                )
                cancellation_requested = state.get("cancellationRequested")
                if not isinstance(cancellation_requested, bool):
                    raise ValueError("persisted cancellation request is invalid")
            else:
                raise ValueError("persisted job state has an unsupported schema")
            creation = _mapping(state.get("creation"), "persisted creation")
            _validate_create_request(creation, self._supported_languages)
            projection = dict(_mapping(state.get("projection"), "persisted projection"))
            _validate_persisted_projection(job_id, creation, projection)
            chunks_root = job_root / "chunks"
            if chunks_root.is_symlink() or not chunks_root.is_dir():
                raise ValueError("persisted chunk storage is unsafe")
            receipts = state.get("receipts")
            if not isinstance(receipts, list):
                raise ValueError("persisted receipts must be an array")
            self._requests[job_id] = deepcopy(dict(creation))
            self._jobs[job_id] = projection
            self._create_keys[job_id] = create_idempotency_key
            if create_idempotency_key is not None:
                if create_idempotency_key in self._created_by_key:
                    raise ValueError("persisted create idempotency key is duplicated")
                self._created_by_key[create_idempotency_key] = job_id
            for raw_receipt in receipts:
                receipt = dict(_mapping(raw_receipt, "persisted receipt"))
                _exact_keys(
                    receipt,
                    {"replayKey", "contentIdentity", "disposition", "acceptedAtUtc"},
                    "persisted receipt",
                )
                replay = _mapping(receipt.get("replayKey"), "persisted replay key")
                _exact_keys(
                    replay,
                    {"schemaVersion", "sessionId", "trackId", "sequenceStart", "sequenceEnd"},
                    "persisted replay key",
                )
                content = _mapping(
                    receipt.get("contentIdentity"),
                    "persisted content identity",
                )
                _exact_keys(
                    content,
                    {"sha256", "byteLength"},
                    "persisted content identity",
                )
                if receipt.get("disposition") != "accepted":
                    raise ValueError("persisted receipt disposition is invalid")
                _utc_timestamp(receipt.get("acceptedAtUtc"), "persisted acceptedAtUtc")
                try:
                    declared_chunk = _find_chunk(
                        creation,
                        track_id=replay.get("trackId"),
                        sequence_start=replay.get("sequenceStart"),
                        sequence_end=replay.get("sequenceEnd"),
                    )
                except KeyError as error:
                    raise ValueError("persisted receipt is not declared") from error
                if replay != _mapping(
                    declared_chunk.get("replayKey"),
                    "declared replay key",
                ) or content != _mapping(
                    declared_chunk.get("contentIdentity"),
                    "declared content identity",
                ):
                    raise ValueError("persisted receipt differs from its declaration")
                receipt_key = _receipt_key(job_id, replay)
                if receipt_key in self._receipts:
                    raise ValueError("persisted receipt is duplicated")
                path = _chunk_path(job_root, replay)
                body = _read_regular_file(path, _MAX_CHUNK_BYTES)
                if (
                    len(body) != content.get("byteLength")
                    or hashlib.sha256(body).hexdigest() != content.get("sha256")
                ):
                    raise ValueError("persisted chunk differs from its receipt")
                self._receipts[receipt_key] = receipt
            status = projection.get("status")
            if status not in _JOB_STATUSES:
                raise ValueError("persisted job status is invalid")
            error = projection.get("error")
            cleanup_was_unverified = (
                status == "failed"
                and isinstance(error, Mapping)
                and error.get("code") == "ASR_CLEANUP_UNVERIFIED"
            )
            if (
                cancellation_requested
                or status == "server_processing"
                or cleanup_was_unverified
            ) and not self._startup_worker_cleanup_verified:
                raise ValueError(
                    "persisted worker state requires verified startup cleanup"
                )
            if cancellation_requested and status != "cancelled":
                self._cancelled.add(job_id)
                projection["status"] = "cancelled"
                projection["updatedAtUtc"] = self._now()
                projection.pop("error", None)
                self._purge_private_audio_locked(job_id)
                continue
            result_path = job_root / "result-revision.json"
            if result_path.exists():
                if status in {"cancelled", "failed"}:
                    _read_regular_file(result_path, _MAX_STATE_BYTES)
                    result_path.unlink()
                elif status in {"server_processing", "complete", "partial"}:
                    result = dict(_read_json_file(result_path))
                    _validate_result_revision(result, projection)
                    if status in {"complete", "partial"} and result.get(
                        "status"
                    ) != status:
                        raise ValueError("persisted result status differs")
                    self._results[job_id] = result
                    if status == "server_processing":
                        projection["status"] = result["status"]
                        projection["updatedAtUtc"] = result["createdAtUtc"]
                        projection.pop("error", None)
                        status = projection["status"]
                        self._persist_job_locked(job_id)
                else:
                    raise ValueError("non-processing job has an unexpected result")
            if status in {"cancelled", "failed"}:
                if status == "cancelled":
                    self._cancelled.add(job_id)
                self._purge_private_audio_locked(job_id)
            if status in {"complete", "partial"} and job_id not in self._results:
                raise ValueError("completed persisted job has no result")
            if status == "server_processing":
                projection["status"] = "failed"
                projection["updatedAtUtc"] = self._now()
                projection["error"] = {
                    "code": "SERVER_RESTARTED",
                    "message": "Server processing was interrupted by a restart.",
                    "retryable": True,
                    "requestId": f"job-{job_id}",
                }
                self._purge_private_audio_locked(job_id)


def _mapping(value: object, field: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise ValueError(f"{field} must be an object")
    return value


def _text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field} must be a non-empty string")
    return value


def _validate_create_request(
    request: Mapping[str, object],
    supported_languages: frozenset[str],
) -> None:
    _exact_keys(
        request,
        {"displayName", "metadata", "tracks", "route", "captureManifest", "chunks"},
        "job",
    )
    display_name = _text(request.get("displayName"), "displayName")
    if len(display_name) > 256 or request.get("route") != "server_batch":
        raise ValueError("invalid display name or route")

    metadata = _mapping(request.get("metadata"), "metadata")
    _exact_keys(
        metadata,
        {
            "sessionId",
            "mode",
            "origin",
            "triggerMode",
            "startedAtUtc",
            "utcOffsetMinutesAtStart",
            "localeHintBcp47",
            "countryCodeHint",
            "preferredLanguagesBcp47",
            "appVersion",
            "platform",
            "privacyPolicyVersion",
            "retentionExpiresAtUtc",
        },
        "metadata",
    )
    session_id = _identifier(metadata.get("sessionId"), 128, "sessionId")
    mode = metadata.get("mode")
    if mode != "meeting":
        raise ValueError("the Phase 5 batch slice accepts meeting sessions only")
    origin = metadata.get("origin")
    if origin != "imported_file":
        raise ValueError("the Phase 5 batch slice accepts imported recordings only")
    if metadata.get("triggerMode") not in {"push_to_talk", "toggle"}:
        raise ValueError("invalid trigger mode")
    started_at = _utc_timestamp(metadata.get("startedAtUtc"), "startedAtUtc")
    utc_offset = metadata.get("utcOffsetMinutesAtStart")
    if utc_offset is not None and not _integer_between(utc_offset, -840, 840):
        raise ValueError("invalid UTC offset")
    locale = metadata.get("localeHintBcp47")
    if locale is not None:
        _language_tag(locale, "localeHintBcp47")
    country = metadata.get("countryCodeHint")
    if country is not None and (
        not isinstance(country, str) or _COUNTRY.fullmatch(country) is None
    ):
        raise ValueError("invalid country code")
    languages = metadata.get("preferredLanguagesBcp47")
    if not isinstance(languages, list) or not 1 <= len(languages) <= 8:
        raise ValueError("an explicit preferred language is required")
    for index, language in enumerate(languages):
        _language_tag(language, f"preferredLanguagesBcp47[{index}]")
    if languages[0].split("-", 1)[0].lower() not in supported_languages:
        raise ValueError("preferred language is unsupported")
    for field, maximum in (
        ("appVersion", 64),
        ("platform", 64),
        ("privacyPolicyVersion", 128),
    ):
        value = _text(metadata.get(field), field)
        if len(value) > maximum:
            raise ValueError(f"{field} is too long")
    retention = metadata.get("retentionExpiresAtUtc")
    if retention is None:
        raise ValueError("server batch metadata requires a retention expiry")
    retention_at = _utc_timestamp(retention, "retentionExpiresAtUtc")
    if retention_at <= started_at:
        raise ValueError("retention expiry must be after session start")
    if retention_at - started_at > _MAX_PRIVATE_RETENTION:
        raise ValueError("retention expiry exceeds the private-storage maximum")

    tracks = request.get("tracks")
    if not isinstance(tracks, list) or not 1 <= len(tracks) <= _MAX_TRACKS:
        raise ValueError("invalid track count")
    track_ids: set[str] = set()
    for value in tracks:
        track = _mapping(value, "tracks[]")
        _exact_keys(
            track,
            {
                "trackId",
                "source",
                "deviceId",
                "originalSampleRateHz",
                "originalChannels",
            },
            "track",
        )
        track_id = _identifier(track.get("trackId"), 64, "trackId")
        if track_id in track_ids:
            raise ValueError("track IDs must be unique")
        track_ids.add(track_id)
        _validate_track_source(track.get("source"), origin)
        device_id = track.get("deviceId")
        if device_id is not None and (
            not isinstance(device_id, str) or not 1 <= len(device_id) <= 128
        ):
            raise ValueError("invalid device ID")
        if not _integer_between(track.get("originalSampleRateHz"), 1, 2**31 - 1):
            raise ValueError("invalid original sample rate")
        if not _integer_between(track.get("originalChannels"), 1, 64):
            raise ValueError("invalid original channel count")

    capture_manifest = _mapping(request.get("captureManifest"), "captureManifest")
    _exact_keys(
        capture_manifest,
        {"schemaVersion", "sessionId", "sha256", "byteLength"},
        "captureManifest",
    )
    if (
        not _integer_between(capture_manifest.get("schemaVersion"), 1, 2**31 - 1)
        or capture_manifest.get("sessionId") != session_id
        or not _valid_sha256(capture_manifest.get("sha256"))
        or not _integer_between(capture_manifest.get("byteLength"), 1, 2**63 - 1)
    ):
        raise ValueError("invalid capture manifest")

    chunks = request.get("chunks")
    if not isinstance(chunks, list) or not 1 <= len(chunks) <= _MAX_CHUNKS:
        raise ValueError("invalid chunk count")
    replay_keys: set[tuple[object, ...]] = set()
    total_bytes = 0
    for value in chunks:
        chunk = _mapping(value, "chunks[]")
        _exact_keys(
            chunk,
            {
                "replayKey",
                "contentIdentity",
                "audioCodec",
                "sampleRateHz",
                "channels",
                "startMs",
                "durationMs",
            },
            "chunk",
        )
        replay = _mapping(chunk.get("replayKey"), "replayKey")
        _exact_keys(
            replay,
            {"schemaVersion", "sessionId", "trackId", "sequenceStart", "sequenceEnd"},
            "replayKey",
        )
        track_id = _identifier(replay.get("trackId"), 64, "replayKey.trackId")
        sequence_start = replay.get("sequenceStart")
        sequence_end = replay.get("sequenceEnd")
        if (
            not _integer_between(replay.get("schemaVersion"), 1, 2**31 - 1)
            or replay.get("sessionId") != session_id
            or track_id not in track_ids
            or not _integer_between(sequence_start, 0, 2**63 - 1)
            or not _integer_between(sequence_end, sequence_start, 2**63 - 1)
        ):
            raise ValueError("invalid replay identity")
        replay_identity = (
            replay["schemaVersion"],
            replay["sessionId"],
            track_id,
            sequence_start,
            sequence_end,
        )
        if replay_identity in replay_keys:
            raise ValueError("replay keys must be unique")
        replay_keys.add(replay_identity)
        content = _mapping(chunk.get("contentIdentity"), "contentIdentity")
        _exact_keys(content, {"sha256", "byteLength"}, "contentIdentity")
        byte_length = content.get("byteLength")
        if (
            not _valid_sha256(content.get("sha256"))
            or not _integer_between(byte_length, 2, _MAX_CHUNK_BYTES)
            or byte_length % 2 != 0
            or chunk.get("audioCodec") != "pcm_s16le"
            or chunk.get("sampleRateHz") != 16000
            or chunk.get("channels") != 1
            or not _integer_between(chunk.get("startMs"), 0, 2**63 - 1)
            or not _integer_between(chunk.get("durationMs"), 1, 2**31 - 1)
        ):
            raise ValueError("invalid chunk declaration")
        if byte_length * 1000 != chunk["durationMs"] * 16000 * 2:
            raise ValueError("chunk duration and byte length differ")
        if sequence_end - sequence_start + 1 != byte_length // 2:
            raise ValueError("chunk sequence range and PCM frame count differ")
        total_bytes += byte_length
        if total_bytes > _MAX_JOB_PCM_BYTES:
            raise ValueError("job audio exceeds the WAV size limit")

    if len(track_ids) != 1:
        raise ValueError("the current batch slice accepts exactly one track")
    _validated_single_track_chunks(chunks)


def _validate_persisted_projection(
    job_id: str,
    creation: Mapping[str, object],
    projection: Mapping[str, object],
) -> None:
    metadata = _mapping(creation.get("metadata"), "metadata")
    status = _text(projection.get("status"), "persisted projection.status")
    if status not in _JOB_STATUSES:
        raise ValueError("persisted job status is invalid")

    keys = {
        "jobId",
        "sessionId",
        "displayName",
        "sessionMode",
        "sessionOrigin",
        "status",
        "route",
        "captureManifest",
        "createdAtUtc",
        "updatedAtUtc",
    }
    if status == "failed":
        keys.add("error")
    _exact_keys(projection, keys, "persisted projection")

    if (
        projection.get("jobId") != job_id
        or projection.get("sessionId") != metadata.get("sessionId")
        or projection.get("displayName") != creation.get("displayName")
        or projection.get("sessionMode") != metadata.get("mode")
        or projection.get("sessionOrigin") != metadata.get("origin")
        or projection.get("route") != creation.get("route")
        or projection.get("captureManifest") != creation.get("captureManifest")
    ):
        raise ValueError("persisted job projection differs from creation")

    created_at = _utc_timestamp(
        projection.get("createdAtUtc"),
        "persisted projection.createdAtUtc",
    )
    updated_at = _utc_timestamp(
        projection.get("updatedAtUtc"),
        "persisted projection.updatedAtUtc",
    )
    if updated_at < created_at:
        raise ValueError("persisted job projection timestamps are invalid")

    if status != "failed":
        return
    error = _mapping(projection.get("error"), "persisted projection.error")
    _exact_keys(
        error,
        {"code", "message", "retryable", "requestId"},
        "persisted projection.error",
    )
    code = _identifier(error.get("code"), 64, "persisted projection.error.code")
    message = _text(error.get("message"), "persisted projection.error.message")
    if (
        code not in _PERSISTED_ERROR_CODES
        or len(message) > 512
        or error.get("retryable") is not True
        or error.get("requestId") != f"job-{job_id}"
    ):
        raise ValueError("persisted job error is invalid")


def _validate_track_source(value: object, origin: object) -> None:
    source = _mapping(value, "track.source")
    kind = source.get("kind")
    if origin == "live_capture":
        _exact_keys(source, {"kind", "source"}, "captured source")
        if kind != "captured" or source.get("source") not in {
            "microphone",
            "system_loopback",
        }:
            raise ValueError("live capture requires a captured source")
        return
    _exact_keys(source, {"kind", "provenance"}, "imported source")
    if kind != "imported":
        raise ValueError("imported sessions require imported sources")
    provenance = source.get("provenance")
    if isinstance(provenance, str) and provenance in {"unknown", "mixed"}:
        return
    declared = _mapping(provenance, "imported provenance")
    _exact_keys(declared, {"kind", "source"}, "imported provenance")
    if declared.get("kind") != "user_declared" or declared.get("source") not in {
        "microphone",
        "system_loopback",
    }:
        raise ValueError("invalid imported source provenance")


def _validate_result_revision(
    result: Mapping[str, object],
    projection: Mapping[str, object],
) -> None:
    _exact_keys(
        result,
        {
            "sessionId",
            "revision",
            "authority",
            "createdAtUtc",
            "captureManifestSha256",
            "previousResultSha256",
            "status",
            "language",
            "transcript",
            "alignedWords",
            "modelProvenance",
        },
        "result revision",
    )
    capture_manifest = _mapping(projection.get("captureManifest"), "captureManifest")
    transcript = _text(result.get("transcript"), "result transcript")
    if (
        result.get("sessionId") != projection.get("sessionId")
        or result.get("revision") != 1
        or result.get("authority") != "server_authoritative"
        or result.get("captureManifestSha256") != capture_manifest.get("sha256")
        or result.get("previousResultSha256") is not None
        or result.get("status") not in {"complete", "partial"}
        or not transcript.strip()
        or len(transcript.encode("utf-8")) > _MAX_TRANSCRIPT_BYTES
        or result.get("alignedWords") != []
    ):
        raise ValueError("result revision identity or content is invalid")
    _utc_timestamp(result.get("createdAtUtc"), "result createdAtUtc")

    language = _mapping(result.get("language"), "result language")
    _exact_keys(language, {"languageBcp47", "confidence"}, "result language")
    _language_tag(language.get("languageBcp47"), "result languageBcp47")
    confidence = language.get("confidence")
    if confidence is not None and (
        isinstance(confidence, bool)
        or not isinstance(confidence, (int, float))
        or not 0 <= confidence <= 1
    ):
        raise ValueError("result language confidence is invalid")

    provenance = result.get("modelProvenance")
    if not isinstance(provenance, list) or len(provenance) != 1:
        raise ValueError("result model provenance is invalid")
    model = _mapping(provenance[0], "result model provenance")
    _exact_keys(
        model,
        {"modelId", "revision", "calibrationRevision"},
        "result model provenance",
    )
    for field in ("modelId", "revision", "calibrationRevision"):
        if (
            len(_text(model.get(field), f"result {field}"))
            > _MAX_MODEL_PROVENANCE_CHARS
        ):
            raise ValueError("result model provenance is oversized")


def _exact_keys(value: Mapping[str, object], expected: set[str], field: str) -> None:
    if set(value) != expected:
        raise ValueError(f"{field} fields differ from the contract")


def _identifier(value: object, maximum: int, field: str) -> str:
    text = _text(value, field)
    if len(text) > maximum or _OPAQUE_ID.fullmatch(text) is None:
        raise ValueError(f"{field} is invalid")
    return text


def _integer_between(value: object, minimum: int, maximum: int) -> bool:
    return (
        isinstance(value, int)
        and not isinstance(value, bool)
        and minimum <= value <= maximum
    )


def _valid_sha256(value: object) -> bool:
    return isinstance(value, str) and _SHA256.fullmatch(value) is not None


def _language_tag(value: object, field: str) -> str:
    text = _text(value, field)
    if len(text) > 35 or _BCP47.fullmatch(text) is None:
        raise ValueError(f"{field} is invalid")
    return text


def _utc_timestamp(value: object, field: str) -> datetime:
    text = _text(value, field)
    if not text.endswith("Z"):
        raise ValueError(f"{field} must be UTC")
    try:
        parsed = datetime.fromisoformat(text[:-1] + "+00:00")
    except ValueError as error:
        raise ValueError(f"{field} is invalid") from error
    if parsed.tzinfo != timezone.utc:
        raise ValueError(f"{field} must be UTC")
    return parsed


def _find_chunk(
    request: Mapping[str, object],
    *,
    track_id: str,
    sequence_start: int,
    sequence_end: int,
) -> Mapping[str, object]:
    chunks = request.get("chunks")
    if not isinstance(chunks, list):
        raise ValueError("chunks must be an array")
    for value in chunks:
        chunk = _mapping(value, "chunks[]")
        replay_key = _mapping(chunk.get("replayKey"), "chunks[].replayKey")
        if (
            replay_key.get("trackId") == track_id
            and replay_key.get("sequenceStart") == sequence_start
            and replay_key.get("sequenceEnd") == sequence_end
        ):
            return chunk
    raise KeyError("declared chunk was not found")


def _chunk_path(job_root: Path, replay_key: Mapping[str, object]) -> Path:
    return job_root / "chunks" / (
        f"{replay_key['trackId']}-{replay_key['sequenceStart']}-"
        f"{replay_key['sequenceEnd']}.pcm"
    )


def _receipt_key(
    job_id: str,
    replay_key: Mapping[str, object],
) -> tuple[object, ...]:
    return (
        job_id,
        replay_key["schemaVersion"],
        replay_key["sessionId"],
        replay_key["trackId"],
        replay_key["sequenceStart"],
        replay_key["sequenceEnd"],
    )


def _validated_single_track_chunks(
    values: list[object],
) -> list[Mapping[str, object]]:
    chunks = [_mapping(value, "chunks[]") for value in values]
    chunks.sort(
        key=lambda chunk: (
            _mapping(chunk.get("replayKey"), "replayKey")["sequenceStart"],
            _mapping(chunk.get("replayKey"), "replayKey")["sequenceEnd"],
        )
    )
    track_ids = {
        _mapping(chunk.get("replayKey"), "replayKey").get("trackId")
        for chunk in chunks
    }
    if len(track_ids) != 1:
        raise ValueError("the Phase 5 ASR slice accepts exactly one audio track")
    expected_start_ms = 0
    expected_sequence_start = 0
    for chunk in chunks:
        replay_key = _mapping(chunk.get("replayKey"), "replayKey")
        start_ms = chunk.get("startMs")
        duration_ms = chunk.get("durationMs")
        content = _mapping(chunk.get("contentIdentity"), "contentIdentity")
        if start_ms != expected_start_ms:
            raise ValueError("the Phase 5 ASR slice requires contiguous chunk timing")
        if not isinstance(duration_ms, int) or isinstance(duration_ms, bool):
            raise ValueError("chunk duration must be an integer")
        byte_length = content.get("byteLength")
        if (
            not isinstance(byte_length, int)
            or isinstance(byte_length, bool)
            or byte_length * 1000 != duration_ms * 16000 * 2
        ):
            raise ValueError("chunk duration does not match mono 16 kHz PCM length")
        sequence_start = replay_key.get("sequenceStart")
        sequence_end = replay_key.get("sequenceEnd")
        if (
            not isinstance(sequence_start, int)
            or not isinstance(sequence_end, int)
            or sequence_end < sequence_start
            or sequence_start != expected_sequence_start
        ):
            raise ValueError("chunk sequence ranges must be contiguous")
        expected_start_ms += duration_ms
        expected_sequence_start = sequence_end + 1
    return chunks


def _selected_language(
    request: Mapping[str, object],
    supported_languages: frozenset[str],
) -> str:
    metadata = _mapping(request.get("metadata"), "metadata")
    languages = metadata.get("preferredLanguagesBcp47")
    if not isinstance(languages, list) or not languages:
        raise ValueError("server batch processing requires an explicit language")
    selected = _text(languages[0], "preferredLanguagesBcp47[0]")
    if selected.split("-", 1)[0].lower() not in supported_languages:
        raise ValueError("selected language is not supported by the locked model")
    return selected


def _publish_wav(destination: Path, chunk_paths: list[Path]) -> None:
    temporary = destination.with_suffix(".wav.part")
    temporary.unlink(missing_ok=True)
    try:
        with wave.open(str(temporary), "wb") as output:
            output.setnchannels(1)
            output.setsampwidth(2)
            output.setframerate(16000)
            for path in chunk_paths:
                output.writeframesraw(path.read_bytes())
        os.replace(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _publish_json(destination: Path, payload: Mapping[str, object]) -> None:
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            dir=destination.parent,
            prefix=".result-",
            delete=False,
        ) as temporary:
            temporary_path = Path(temporary.name)
            json.dump(payload, temporary, ensure_ascii=True, separators=(",", ":"))
            temporary.write("\n")
            temporary.flush()
            os.fsync(temporary.fileno())
        os.replace(temporary_path, destination)
        temporary_path = None
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)


def _read_json_file(path: Path) -> dict[str, object]:
    body = _read_regular_file(path, _MAX_STATE_BYTES)
    try:
        value = json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"persisted JSON is invalid: {path.name}") from error
    if not isinstance(value, dict):
        raise ValueError(f"persisted JSON must be an object: {path.name}")
    return value


def _unlink_private_regular_file(path: Path, label: str) -> None:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        return
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"{label} is unsafe")
    path.unlink()


def _read_regular_file(path: Path, maximum_bytes: int) -> bytes:
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise ValueError(f"required persisted file is missing: {path.name}") from error
    if (
        stat.S_ISLNK(metadata.st_mode)
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_size > maximum_bytes
    ):
        raise ValueError(f"persisted file is unsafe or oversized: {path.name}")
    return path.read_bytes()
