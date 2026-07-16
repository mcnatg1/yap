from __future__ import annotations

from pathlib import Path
import threading
from typing import Callable

from yap_server.pools.batch_contract import WorkerContainmentError

from .artifacts import publish_json
from .contract_values import (
    MAX_MODEL_PROVENANCE_CHARS,
    MAX_TRANSCRIPT_BYTES,
    mapping,
    text,
)
from .job_store import DurableJobState, RecordingJobStore
from .result_contract import validate_result_revision


class JobCompletionCoordinator:
    """Converges one worker future into a durable result or safe tombstone."""

    def __init__(
        self,
        *,
        storage_root: Path,
        state: DurableJobState,
        store: RecordingJobStore,
        futures: dict[str, object],
        completion_events: dict[str, threading.Event],
        lock: threading.RLock,
        now: Callable[[], str],
    ) -> None:
        self._storage_root = storage_root
        self._state = state
        self._store = store
        self._futures = futures
        self._completion_events = completion_events
        self._lock = lock
        self._now = now

    def finish_safely(
        self,
        job_id: str,
        language_bcp47: str,
        future: object,
        completion_event: threading.Event,
    ) -> None:
        try:
            self._finish(job_id, language_bcp47, future)
        except Exception:
            # Future callbacks are an outer trust boundary. Never let a storage
            # exception reach concurrent.futures' default callback logger,
            # which would print filesystem details. Preserve an already
            # published complete result for restart reconciliation; otherwise
            # converge to the existing generic retryable failure tombstone.
            try:
                with self._lock:
                    self._discard_future(job_id, future)
                    job = self._state.jobs.get(job_id)
                    if job is None or job.get("status") in {"complete", "partial"}:
                        return
                    if job_id not in self._state.cancelled and job.get("status") != "failed":
                        job["status"] = "failed"
                        job["updatedAtUtc"] = self._now()
                        job["error"] = {
                            "code": "SERVER_STORAGE_ERROR",
                            "message": "Private result storage did not complete safely.",
                            "retryable": True,
                            "requestId": f"job-{job_id}",
                        }
                    try:
                        self._store.purge_private_audio(self._state, job_id)
                    except Exception:
                        pass
            except Exception:
                pass
        finally:
            completion_event.set()
            with self._lock:
                if self._completion_events.get(job_id) is completion_event:
                    self._completion_events.pop(job_id, None)

    def _finish(self, job_id: str, language_bcp47: str, future: object) -> None:
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
            result, created_at = self._result_from_worker(
                job_id,
                language_bcp47,
                payload,
                future,
            )
        except (KeyError, TypeError, ValueError):
            self._mark_failed_unless_cancelled(
                job_id,
                future,
                code="ASR_RESULT_INVALID",
                message="The private ASR worker returned an invalid result.",
            )
            return
        if result is None:
            return
        result_path = self._storage_root / "jobs" / job_id / "result-revision.json"
        try:
            publish_json(result_path, result)
        except OSError:
            self._mark_failed_unless_cancelled(
                job_id,
                future,
                code="ASR_RESULT_PUBLISH_FAILED",
                message="The private ASR result could not be stored safely.",
            )
            return
        with self._lock:
            if job_id in self._state.cancelled:
                self._discard_future(job_id, future)
                self._store.purge_private_audio(self._state, job_id)
                return
            self._state.results[job_id] = result
            job = self._state.jobs[job_id]
            job["status"] = "complete"
            job["updatedAtUtc"] = created_at
            self._discard_future(job_id, future)
            self._store.persist(self._state, job_id)

    def _result_from_worker(
        self,
        job_id: str,
        language_bcp47: str,
        payload: object,
        future: object,
    ) -> tuple[dict[str, object] | None, str]:
        worker_payload = mapping(payload, "worker result")
        transcript = mapping(worker_payload.get("transcript"), "worker transcript")
        model = mapping(worker_payload.get("model"), "worker model")
        transcript_text = text(transcript.get("text"), "worker transcript.text")
        if (
            not transcript_text.strip()
            or len(transcript_text.encode("utf-8")) > MAX_TRANSCRIPT_BYTES
        ):
            raise ValueError("worker transcript is empty or oversized")
        model_id = text(model.get("id"), "worker model.id")
        model_revision = text(model.get("revision"), "worker model.revision")
        if (
            len(model_id) > MAX_MODEL_PROVENANCE_CHARS
            or len(model_revision) > MAX_MODEL_PROVENANCE_CHARS
        ):
            raise ValueError("worker model identity is oversized")
        created_at = self._now()
        with self._lock:
            if job_id in self._state.cancelled:
                self._discard_future(job_id, future)
                self._store.purge_private_audio(self._state, job_id)
                return None, created_at
            job = self._state.jobs[job_id]
            capture_manifest = mapping(job["captureManifest"], "captureManifest")
            result: dict[str, object] = {
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
            validate_result_revision(result, job)
        return result, created_at

    def _mark_containment_unverified(self, job_id: str, future: object) -> None:
        failed_at = self._now()
        with self._lock:
            self._discard_future(job_id, future)
            job = self._state.jobs[job_id]
            job["status"] = "failed"
            job["updatedAtUtc"] = failed_at
            job["error"] = {
                "code": "ASR_CLEANUP_UNVERIFIED",
                "message": "The private ASR worker cleanup could not be verified.",
                "retryable": True,
                "requestId": f"job-{job_id}",
            }
            self._store.purge_private_audio(self._state, job_id)

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
            self._discard_future(job_id, future)
            if job_id in self._state.cancelled:
                self._store.purge_private_audio(self._state, job_id)
                return
            job = self._state.jobs[job_id]
            job["status"] = "failed"
            job["updatedAtUtc"] = failed_at
            job["error"] = {
                "code": code,
                "message": message,
                "retryable": True,
                "requestId": f"job-{job_id}",
            }
            self._store.purge_private_audio(self._state, job_id)

    def _discard_future(self, job_id: str, future: object) -> None:
        if self._futures.get(job_id) is future:
            self._futures.pop(job_id, None)
