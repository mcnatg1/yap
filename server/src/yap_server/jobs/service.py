from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass
from datetime import datetime
import hashlib
import os
from pathlib import Path
import re
import shutil
import stat
import tempfile
import threading
from typing import Callable, Mapping, Protocol, Sequence
from uuid import uuid4

from yap_server.pools.batch_asr import (
    BatchAsrJob,
    PoolBackpressure,
    WorkerContainmentError,
)
from .artifacts import (
    MAX_STATE_BYTES as _MAX_STATE_BYTES,
    publish_json as _publish_json,
    publish_wav as _publish_wav,
    read_json_file as _read_json_file,
    read_regular_file as _read_regular_file,
    sha256_file as _sha256_file,
    unlink_private_regular_file as _unlink_private_regular_file,
)
from .chunk_contract import (
    chunk_path as _chunk_path,
    find_chunk as _find_chunk,
    receipt_key as _receipt_key,
    validated_single_track_chunks as _validated_single_track_chunks,
)
from .contract_values import (
    JOB_STATUSES as _JOB_STATUSES,
    MAX_CHUNK_BYTES as _MAX_CHUNK_BYTES,
    MAX_CLIENT_CLOCK_SKEW as _MAX_CLIENT_CLOCK_SKEW,
    MAX_MODEL_PROVENANCE_CHARS as _MAX_MODEL_PROVENANCE_CHARS,
    MAX_TRANSCRIPT_BYTES as _MAX_TRANSCRIPT_BYTES,
    TERMINAL_STATUSES as _TERMINAL_STATUSES,
    exact_keys as _exact_keys,
    identifier as _identifier,
    mapping as _mapping,
    text as _text,
    utc_timestamp as _utc_timestamp,
)
from .intake_contract import (
    selected_language as _selected_language,
    validate_create_request as _validate_create_request,
)
from .result_contract import (
    validate_persisted_projection as _validate_persisted_projection,
    validate_result_revision as _validate_result_revision,
)


_MAX_STORED_JOBS = 512
_CANCELLATION_ACK_TIMEOUT_SECONDS = 2.0
_JOB_DIRECTORY = re.compile(r"^job-[0-9a-f]{32}$")


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
