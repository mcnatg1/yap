from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass, field
import hashlib
from pathlib import Path
import re
import shutil
import stat
from typing import Callable, Mapping, Sequence

from .artifacts import (
    MAX_STATE_BYTES,
    publish_json,
    read_json_file,
    read_regular_file,
    unlink_private_regular_file,
)
from .chunk_contract import chunk_path, find_chunk, receipt_key
from .contract_values import (
    JOB_STATUSES,
    MAX_CHUNK_BYTES,
    exact_keys,
    mapping,
    utc_timestamp,
)
from .intake_contract import validate_create_request
from .result_contract import validate_persisted_projection, validate_result_revision
from .state_schema import persisted_state_metadata


_JOB_DIRECTORY = re.compile(r"^job-[0-9a-f]{32}$")


@dataclass(slots=True)
class DurableJobState:
    jobs: dict[str, dict[str, object]] = field(default_factory=dict)
    requests: dict[str, dict[str, object]] = field(default_factory=dict)
    results: dict[str, dict[str, object]] = field(default_factory=dict)
    receipts: dict[tuple[object, ...], dict[str, object]] = field(default_factory=dict)
    cancelled: set[str] = field(default_factory=set)
    create_keys: dict[str, str | None] = field(default_factory=dict)
    created_by_key: dict[str, str] = field(default_factory=dict)


class RecordingJobStore:
    """Persists and recovers the durable half of the recording-job aggregate.

    The lifecycle service serializes mutations before calling this adapter. Runtime
    worker futures and commit coordination intentionally remain outside this state.
    """

    def __init__(
        self,
        storage_root: Path,
        *,
        supported_languages: Sequence[str],
        now: Callable[[], str],
        startup_worker_cleanup_verified: bool,
    ) -> None:
        self.root = storage_root.resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        self._supported_languages = frozenset(supported_languages)
        self._now = now
        self._startup_worker_cleanup_verified = startup_worker_cleanup_verified

    def load(self) -> DurableJobState:
        state = DurableJobState()
        jobs_root = self.root / "jobs"
        if not jobs_root.exists():
            return state
        if jobs_root.is_symlink() or not jobs_root.is_dir():
            raise ValueError("job storage root must be a real directory")
        for job_root in sorted(jobs_root.iterdir(), key=lambda path: path.name):
            self._load_job(state, job_root)
        return state

    def persist(self, state: DurableJobState, job_id: str) -> None:
        receipts = [
            deepcopy(receipt)
            for key, receipt in state.receipts.items()
            if key[0] == job_id
        ]
        receipts.sort(
            key=lambda receipt: (
                receipt["replayKey"]["trackId"],
                receipt["replayKey"]["sequenceStart"],
                receipt["replayKey"]["sequenceEnd"],
            )
        )
        publish_json(
            self.root / "jobs" / job_id / "state.json",
            {
                "schemaVersion": 3,
                "createIdempotencyKey": state.create_keys[job_id],
                "cancellationRequested": job_id in state.cancelled,
                "creation": state.requests[job_id],
                "projection": state.jobs[job_id],
                "receipts": receipts,
            },
        )

    def purge_private_audio(self, state: DurableJobState, job_id: str) -> None:
        job_root = self.root / "jobs" / job_id
        chunks_root = job_root / "chunks"
        chunk_metadata = chunks_root.lstat()
        if stat.S_ISLNK(chunk_metadata.st_mode) or not stat.S_ISDIR(
            chunk_metadata.st_mode
        ):
            raise ValueError("private chunk storage is unsafe")
        for stored_receipt_key in tuple(state.receipts):
            if stored_receipt_key[0] == job_id:
                state.receipts.pop(stored_receipt_key, None)
        state.results.pop(job_id, None)
        self.persist(state, job_id)
        for entry in chunks_root.iterdir():
            unlink_private_regular_file(entry, "private recording chunk")
        for name in (
            "input.wav",
            "input.wav.part",
            "worker-result.json",
            "result-revision.json",
        ):
            unlink_private_regular_file(job_root / name, "private recording artifact")

    def delete(self, state: DurableJobState, job_id: str) -> None:
        job_root = self.root / "jobs" / job_id
        metadata = job_root.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
            raise ValueError("expired job storage is unsafe")
        shutil.rmtree(job_root)
        state.jobs.pop(job_id, None)
        state.requests.pop(job_id, None)
        state.results.pop(job_id, None)
        state.cancelled.discard(job_id)
        create_key = state.create_keys.pop(job_id, None)
        if create_key is not None and state.created_by_key.get(create_key) == job_id:
            state.created_by_key.pop(create_key, None)
        for stored_receipt_key in tuple(state.receipts):
            if stored_receipt_key[0] == job_id:
                state.receipts.pop(stored_receipt_key, None)

    def _load_job(self, state: DurableJobState, job_root: Path) -> None:
        if job_root.is_symlink() or not job_root.is_dir():
            raise ValueError("job storage contains an unsafe entry")
        job_id = job_root.name
        if _JOB_DIRECTORY.fullmatch(job_id) is None:
            raise ValueError("job storage contains an invalid job directory")
        persisted = read_json_file(job_root / "state.json")
        create_idempotency_key, cancellation_requested = persisted_state_metadata(persisted)
        creation = mapping(persisted.get("creation"), "persisted creation")
        validate_create_request(creation, self._supported_languages)
        projection = dict(mapping(persisted.get("projection"), "persisted projection"))
        validate_persisted_projection(job_id, creation, projection)
        chunks_root = job_root / "chunks"
        if chunks_root.is_symlink() or not chunks_root.is_dir():
            raise ValueError("persisted chunk storage is unsafe")
        receipts = persisted.get("receipts")
        if not isinstance(receipts, list):
            raise ValueError("persisted receipts must be an array")
        state.requests[job_id] = deepcopy(dict(creation))
        state.jobs[job_id] = projection
        state.create_keys[job_id] = create_idempotency_key
        if create_idempotency_key is not None:
            if create_idempotency_key in state.created_by_key:
                raise ValueError("persisted create idempotency key is duplicated")
            state.created_by_key[create_idempotency_key] = job_id
        for raw_receipt in receipts:
            self._load_receipt(state, job_id, job_root, creation, raw_receipt)
        self._reconcile_projection(
            state,
            job_id,
            job_root,
            projection,
            cancellation_requested,
        )

    def _load_receipt(
        self,
        state: DurableJobState,
        job_id: str,
        job_root: Path,
        creation: Mapping[str, object],
        raw_receipt: object,
    ) -> None:
        receipt = dict(mapping(raw_receipt, "persisted receipt"))
        exact_keys(
            receipt,
            {"replayKey", "contentIdentity", "disposition", "acceptedAtUtc"},
            "persisted receipt",
        )
        replay = mapping(receipt.get("replayKey"), "persisted replay key")
        exact_keys(
            replay,
            {"schemaVersion", "sessionId", "trackId", "sequenceStart", "sequenceEnd"},
            "persisted replay key",
        )
        content = mapping(receipt.get("contentIdentity"), "persisted content identity")
        exact_keys(content, {"sha256", "byteLength"}, "persisted content identity")
        if receipt.get("disposition") != "accepted":
            raise ValueError("persisted receipt disposition is invalid")
        utc_timestamp(receipt.get("acceptedAtUtc"), "persisted acceptedAtUtc")
        try:
            declared_chunk = find_chunk(
                creation,
                track_id=replay.get("trackId"),
                sequence_start=replay.get("sequenceStart"),
                sequence_end=replay.get("sequenceEnd"),
            )
        except KeyError as error:
            raise ValueError("persisted receipt is not declared") from error
        if replay != mapping(
            declared_chunk.get("replayKey"),
            "declared replay key",
        ) or content != mapping(
            declared_chunk.get("contentIdentity"),
            "declared content identity",
        ):
            raise ValueError("persisted receipt differs from its declaration")
        key = receipt_key(job_id, replay)
        if key in state.receipts:
            raise ValueError("persisted receipt is duplicated")
        body = read_regular_file(chunk_path(job_root, replay), MAX_CHUNK_BYTES)
        if (
            len(body) != content.get("byteLength")
            or hashlib.sha256(body).hexdigest() != content.get("sha256")
        ):
            raise ValueError("persisted chunk differs from its receipt")
        state.receipts[key] = receipt

    def _reconcile_projection(
        self,
        state: DurableJobState,
        job_id: str,
        job_root: Path,
        projection: dict[str, object],
        cancellation_requested: bool,
    ) -> None:
        status = projection.get("status")
        if status not in JOB_STATUSES:
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
            raise ValueError("persisted worker state requires verified startup cleanup")
        if cancellation_requested and status != "cancelled":
            state.cancelled.add(job_id)
            projection["status"] = "cancelled"
            projection["updatedAtUtc"] = self._now()
            projection.pop("error", None)
            self.purge_private_audio(state, job_id)
            return
        status = self._load_result(state, job_id, job_root, projection, status)
        if status in {"cancelled", "failed"}:
            if status == "cancelled":
                state.cancelled.add(job_id)
            self.purge_private_audio(state, job_id)
        if status in {"complete", "partial"} and job_id not in state.results:
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
            self.purge_private_audio(state, job_id)

    def _load_result(
        self,
        state: DurableJobState,
        job_id: str,
        job_root: Path,
        projection: dict[str, object],
        status: object,
    ) -> object:
        result_path = job_root / "result-revision.json"
        if not result_path.exists():
            return status
        if status in {"cancelled", "failed"}:
            read_regular_file(result_path, MAX_STATE_BYTES)
            result_path.unlink()
            return status
        if status not in {"server_processing", "complete", "partial"}:
            raise ValueError("non-processing job has an unexpected result")
        result = dict(read_json_file(result_path))
        validate_result_revision(result, projection)
        if status in {"complete", "partial"} and result.get("status") != status:
            raise ValueError("persisted result status differs")
        state.results[job_id] = result
        if status == "server_processing":
            projection["status"] = result["status"]
            projection["updatedAtUtc"] = result["createdAtUtc"]
            projection.pop("error", None)
            status = projection["status"]
            self.persist(state, job_id)
        return status
