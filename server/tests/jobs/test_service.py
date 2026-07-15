from __future__ import annotations

import json
import tempfile
import threading
import time
import unittest
from concurrent.futures import Future
import hashlib
from pathlib import Path
import wave
from unittest.mock import patch

from yap_server.jobs import JobServiceError, RecordingJobService
from yap_server.pools.batch_asr import (
    BatchAsrJob,
    BatchAsrPool,
    PoolBackpressure,
    WorkerContainmentError,
    WorkerExecutionError,
)
from yap_server.pools.batch_asr_worker import MAX_AUDIO_SECONDS, SAMPLE_RATE_HZ


class _Processor:
    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        raise AssertionError(f"job {job.job_id} must not dispatch before commit")


class _ControlledProcessor:
    def __init__(self) -> None:
        self.jobs: list[BatchAsrJob] = []
        self.future: Future[dict[str, object]] = Future()

    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        self.jobs.append(job)
        return self.future


class _BusyProcessor:
    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        raise PoolBackpressure(f"capacity unavailable for {job.job_id}")


class _UnstoppableProcessor:
    def __init__(self) -> None:
        self.future: Future[dict[str, object]] = Future()

    def submit(self, _job: BatchAsrJob) -> Future[dict[str, object]]:
        self.future.set_running_or_notify_cancel()
        return self.future

    def cancel(self, _job_id: str) -> bool:
        return False


class _ActiveCancellationWorker:
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
            raise AssertionError(f"active job {job.job_id} was not cancelled")
        self.stopped.set()
        raise WorkerExecutionError("isolated ASR worker was cancelled")


class _UnverifiedCleanupWorker:
    def __init__(self) -> None:
        self.started = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        if not cancellation.wait(timeout=5):
            raise AssertionError(f"active job {job.job_id} was not cancelled")
        raise WorkerContainmentError("owned container cleanup could not be verified")


class _DelayedCancellationWorker:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.cancellation_received = threading.Event()
        self.release_cleanup = threading.Event()

    def run(
        self,
        job: BatchAsrJob,
        cancellation: threading.Event,
    ) -> dict[str, object]:
        self.started.set()
        if not cancellation.wait(timeout=5):
            raise AssertionError(f"active job {job.job_id} was not cancelled")
        self.cancellation_received.set()
        if not self.release_cleanup.wait(timeout=5):
            raise AssertionError(f"active job {job.job_id} cleanup was not released")
        raise WorkerExecutionError("isolated ASR worker was cancelled")


def _create_request(
    *,
    session_id: str = "s-phase5-create",
    retention_expires_at_utc: str | None = "2026-08-13T21:00:00Z",
) -> dict[str, object]:
    track_id = "track-1"
    chunk = bytes(320)
    return {
        "displayName": "Phase 5 vertical slice",
        "metadata": {
            "sessionId": session_id,
            "mode": "meeting",
            "origin": "imported_file",
            "triggerMode": "toggle",
            "startedAtUtc": "2026-07-14T21:00:00Z",
            "utcOffsetMinutesAtStart": -300,
            "localeHintBcp47": "en-US",
            "countryCodeHint": "US",
            "preferredLanguagesBcp47": ["en-US"],
            "appVersion": "0.1.0",
            "platform": "windows",
            "privacyPolicyVersion": "development-only",
            "retentionExpiresAtUtc": retention_expires_at_utc,
        },
        "tracks": [
            {
                "trackId": track_id,
                "source": {"kind": "imported", "provenance": "unknown"},
                "deviceId": None,
                "originalSampleRateHz": 16000,
                "originalChannels": 1,
            }
        ],
        "route": "server_batch",
        "captureManifest": {
            "schemaVersion": 1,
            "sessionId": session_id,
            "sha256": "a" * 64,
            "byteLength": 4096,
        },
        "chunks": [
            {
                "replayKey": {
                    "schemaVersion": 1,
                    "sessionId": session_id,
                    "trackId": track_id,
                    "sequenceStart": 0,
                    "sequenceEnd": 159,
                },
                "contentIdentity": {
                    "sha256": hashlib.sha256(chunk).hexdigest(),
                    "byteLength": len(chunk),
                },
                "audioCodec": "pcm_s16le",
                "sampleRateHz": 16000,
                "channels": 1,
                "startMs": 0,
                "durationMs": 10,
            }
        ],
    }


def _published_result(job: dict[str, object]) -> dict[str, object]:
    return {
        "sessionId": job["sessionId"],
        "revision": 1,
        "authority": "server_authoritative",
        "createdAtUtc": "2026-07-14T21:20:00Z",
        "captureManifestSha256": job["captureManifest"]["sha256"],
        "previousResultSha256": None,
        "status": "complete",
        "language": {"languageBcp47": "en", "confidence": None},
        "transcript": "Crash-safe private transcript.",
        "alignedWords": [],
        "modelProvenance": [
            {
                "modelId": "private-asr",
                "revision": "revision-1",
                "calibrationRevision": "asr-not-applicable",
            }
        ],
    }


class RecordingJobServiceTests(unittest.TestCase):
    def test_intake_duration_limit_matches_the_isolated_worker(self) -> None:
        from yap_server.jobs.contract_values import MAX_JOB_PCM_BYTES

        self.assertEqual(
            MAX_JOB_PCM_BYTES,
            SAMPLE_RATE_HZ * 2 * MAX_AUDIO_SECONDS,
        )

    def test_meeting_intake_requires_finite_retention_after_capture_start(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:00:00Z",
            )

            for retention in (
                None,
                "2026-07-14T20:59:59Z",
                "2026-08-13T21:00:01Z",
            ):
                with self.subTest(retention=retention):
                    with self.assertRaises(JobServiceError) as invalid:
                        service.create(
                            _create_request(retention_expires_at_utc=retention)
                        )
                    self.assertEqual(invalid.exception.status, 400)
                    self.assertEqual(invalid.exception.code, "INVALID_JOB")

    def test_intake_rejects_retention_that_is_already_expired_by_server_time(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-15T21:00:00Z",
            )

            with self.assertRaises(JobServiceError) as invalid:
                service.create(
                    _create_request(
                        retention_expires_at_utc="2026-07-15T20:59:59Z"
                    )
                )

            self.assertEqual(invalid.exception.status, 400)
            self.assertEqual(invalid.exception.code, "INVALID_JOB")

    def test_intake_rejects_a_sequence_range_that_does_not_cover_the_pcm_frames(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:00:00Z",
            )
            request = _create_request()
            request["chunks"][0]["replayKey"]["sequenceEnd"] = 0

            with self.assertRaises(JobServiceError) as invalid:
                service.create(request)

            self.assertEqual(invalid.exception.status, 400)
            self.assertEqual(invalid.exception.code, "INVALID_JOB")

    def test_intake_rejects_session_shapes_outside_the_imported_meeting_slice(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:00:00Z",
            )

            for field, value in (("mode", "dictation"), ("origin", "live_capture")):
                with self.subTest(field=field, value=value):
                    request = _create_request()
                    request["metadata"][field] = value
                    with self.assertRaises(JobServiceError) as invalid:
                        service.create(request)
                    self.assertEqual(invalid.exception.status, 400)
                    self.assertEqual(invalid.exception.code, "INVALID_JOB")

    def test_intake_requires_chunk_sequences_to_begin_at_zero(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:00:00Z",
            )
            request = _create_request()
            request["chunks"][0]["replayKey"]["sequenceStart"] = 1
            request["chunks"][0]["replayKey"]["sequenceEnd"] = 160

            with self.assertRaises(JobServiceError) as invalid:
                service.create(request)

            self.assertEqual(invalid.exception.status, 400)
            self.assertEqual(invalid.exception.code, "INVALID_JOB")

    def test_result_model_provenance_matches_the_openapi_256_character_bound(self) -> None:
        from yap_server.jobs.result_contract import validate_result_revision

        projection = {
            "sessionId": "s-phase5-create",
            "captureManifest": {"sha256": "a" * 64},
        }
        result = _published_result(projection)
        result["modelProvenance"][0]["modelId"] = "m" * 257

        with self.assertRaises(ValueError):
            validate_result_revision(result, projection)

    def test_create_returns_and_replays_the_immutable_job_projection(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:01:00Z",
            )

            created = service.create(_create_request())

            self.assertRegex(created["jobId"], r"^job-[0-9a-f]{32}$")
            self.assertEqual(created["sessionId"], "s-phase5-create")
            self.assertEqual(created["displayName"], "Phase 5 vertical slice")
            self.assertEqual(created["sessionMode"], "meeting")
            self.assertEqual(created["sessionOrigin"], "imported_file")
            self.assertEqual(created["status"], "accepted")
            self.assertEqual(created["route"], "server_batch")
            self.assertEqual(created["captureManifest"], _create_request()["captureManifest"])
            self.assertEqual(created["createdAtUtc"], "2026-07-14T21:01:00Z")
            self.assertEqual(created["updatedAtUtc"], "2026-07-14T21:01:00Z")
            self.assertEqual(service.get(created["jobId"]), created)

    def test_create_idempotency_survives_restart_and_rejects_conflicting_content(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:01:00Z",
            )
            request = _create_request()

            created = service.create(request, idempotency_key="job-client-1")
            replayed = service.create(request, idempotency_key="job-client-1")

            self.assertEqual(replayed, created)
            conflicting = _create_request()
            conflicting["displayName"] = "different recording"
            with self.assertRaises(JobServiceError) as conflict:
                service.create(conflicting, idempotency_key="job-client-1")
            self.assertEqual(conflict.exception.status, 409)
            self.assertEqual(conflict.exception.code, "CREATE_IDEMPOTENCY_CONFLICT")

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:02:00Z",
            )
            self.assertEqual(
                restarted.create(request, idempotency_key="job-client-1"),
                created,
            )
            self.assertEqual(len(list((root / "jobs").iterdir())), 1)

    def test_create_persistence_failure_rolls_back_before_retry_and_restart(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:01:00Z",
            )
            request = _create_request()

            from yap_server.jobs import job_store as store_module

            with patch.object(
                store_module,
                "publish_json",
                side_effect=OSError("private state storage unavailable"),
            ):
                with self.assertRaises(OSError):
                    service.create(request, idempotency_key="job-client-retry")

            self.assertEqual(list((root / "jobs").iterdir()), [])
            created = service.create(
                request,
                idempotency_key="job-client-retry",
            )
            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:02:00Z",
            )

            self.assertEqual(
                restarted.create(request, idempotency_key="job-client-retry"),
                created,
            )
            self.assertEqual(len(list((root / "jobs").iterdir())), 1)

    def test_declared_chunk_is_hash_verified_and_published_atomically(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:02:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            digest = hashlib.sha256(chunk).hexdigest()

            plan = service.prepare_chunk_upload(
                created["jobId"],
                track_id="track-1",
                sequence_start=0,
                sequence_end=159,
                idempotency_key="1/s-phase5-create/track-1/0/159",
                content_sha256=digest,
                audio_codec="pcm_s16le",
                sample_rate_hz=16000,
                channels=1,
                content_length=len(chunk),
            )
            receipt = service.accept_chunk(plan, chunk)

            self.assertEqual(receipt["replayKey"], request["chunks"][0]["replayKey"])
            self.assertEqual(receipt["contentIdentity"], request["chunks"][0]["contentIdentity"])
            self.assertEqual(receipt["disposition"], "accepted")
            self.assertEqual(receipt["acceptedAtUtc"], "2026-07-14T21:02:00Z")
            self.assertEqual(service.get(created["jobId"])["status"], "uploading")

    def test_commit_builds_worker_wav_and_publishes_an_immutable_result(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            processor = _ControlledProcessor()
            service = RecordingJobService(
                Path(temporary),
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:03:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            plan = service.prepare_chunk_upload(
                created["jobId"],
                track_id="track-1",
                sequence_start=0,
                sequence_end=159,
                idempotency_key="1/s-phase5-create/track-1/0/159",
                content_sha256=hashlib.sha256(chunk).hexdigest(),
                audio_codec="pcm_s16le",
                sample_rate_hz=16000,
                channels=1,
                content_length=len(chunk),
            )
            service.accept_chunk(plan, chunk)

            committed = service.commit(
                created["jobId"],
                {
                    "captureManifest": request["captureManifest"],
                    "chunkCount": 1,
                },
            )

            self.assertEqual(committed["status"], "server_processing")
            self.assertEqual(len(processor.jobs), 1)
            worker_job = processor.jobs[0]
            self.assertEqual(worker_job.job_id, created["jobId"])
            self.assertEqual(worker_job.language, "en")
            with wave.open(str(worker_job.input_path), "rb") as audio:
                self.assertEqual(audio.getnchannels(), 1)
                self.assertEqual(audio.getframerate(), 16000)
                self.assertEqual(audio.getsampwidth(), 2)
                self.assertEqual(audio.readframes(audio.getnframes()), chunk)

            processor.future.set_result(
                {
                    "schemaVersion": 1,
                    "jobId": created["jobId"],
                    "model": {
                        "poolId": "cohere-batch",
                        "id": "CohereLabs/cohere-transcribe-03-2026",
                        "revision": "b1eacc2686a3d08ceaae5f24a88b1d519620bc09",
                    },
                    "audio": {
                        "sha256": worker_job.input_sha256,
                        "sampleRateHz": 16000,
                        "durationMs": 10,
                    },
                    "transcript": {
                        "text": "Phase five is connected.",
                        "language": "en",
                        "punctuation": True,
                    },
                }
            )

            self.assertEqual(service.get(created["jobId"])["status"], "complete")
            self.assertEqual(
                service.get_result(created["jobId"]),
                {
                    "sessionId": "s-phase5-create",
                    "revision": 1,
                    "authority": "server_authoritative",
                    "createdAtUtc": "2026-07-14T21:03:00Z",
                    "captureManifestSha256": "a" * 64,
                    "previousResultSha256": None,
                    "status": "complete",
                    "language": {
                        "languageBcp47": "en-US",
                        "confidence": None,
                    },
                    "transcript": "Phase five is connected.",
                    "alignedWords": [],
                    "modelProvenance": [
                        {
                            "modelId": "CohereLabs/cohere-transcribe-03-2026",
                            "revision": "b1eacc2686a3d08ceaae5f24a88b1d519620bc09",
                            "calibrationRevision": "asr-not-applicable",
                        }
                    ],
                },
            )

    def test_processing_intent_failure_prevents_worker_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            processor = _ControlledProcessor()
            service = RecordingJobService(
                Path(temporary),
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:03:15Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )

            from yap_server.jobs import job_store as store_module

            original_publish_json = store_module.publish_json

            def fail_processing_state(path: Path, value: object) -> None:
                if (
                    path.name == "state.json"
                    and value["projection"]["status"] == "server_processing"
                ):
                    raise OSError("private processing state unavailable")
                original_publish_json(path, value)

            with patch.object(store_module, "publish_json", fail_processing_state):
                with self.assertRaises(OSError):
                    service.commit(
                        created["jobId"],
                        {
                            "captureManifest": request["captureManifest"],
                            "chunkCount": 1,
                        },
                    )

            self.assertEqual(processor.jobs, [])
            self.assertEqual(service.get(created["jobId"])["status"], "uploading")
            with self.assertRaises(JobServiceError) as unavailable:
                service.get_result(created["jobId"])
            self.assertEqual(unavailable.exception.code, "RESULT_NOT_READY")

    def test_delete_after_completion_purges_private_artifacts_and_returns_cancelled(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            processor = _ControlledProcessor()
            service = RecordingJobService(
                root,
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:03:30Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            service.commit(
                created["jobId"],
                {
                    "captureManifest": request["captureManifest"],
                    "chunkCount": 1,
                },
            )
            processor.future.set_result(
                {
                    "model": {"id": "private-asr", "revision": "revision-1"},
                    "transcript": {"text": "Private terminal transcript."},
                }
            )
            job_root = root / "jobs" / created["jobId"]
            self.assertTrue((job_root / "input.wav").is_file())
            self.assertTrue((job_root / "result-revision.json").is_file())
            self.assertNotEqual(list((job_root / "chunks").iterdir()), [])

            cancelled = service.cancel(created["jobId"])
            replayed = service.cancel(created["jobId"])

            self.assertEqual(cancelled["status"], "cancelled")
            self.assertNotIn("error", cancelled)
            self.assertEqual(replayed, cancelled)
            with self.assertRaises(JobServiceError) as missing_result:
                service.get_result(created["jobId"])
            self.assertEqual(missing_result.exception.code, "RESULT_NOT_READY")
            self.assertEqual(list((job_root / "chunks").iterdir()), [])
            for name in (
                "input.wav",
                "input.wav.part",
                "worker-result.json",
                "result-revision.json",
            ):
                self.assertFalse((job_root / name).exists())

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:03:31Z",
                startup_worker_cleanup_verified=True,
            )
            self.assertEqual(restarted.get(created["jobId"]), cancelled)

    def test_completion_state_failure_is_private_and_restart_recovers_result(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            processor = _ControlledProcessor()
            service = RecordingJobService(
                root,
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:03:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            service.commit(
                created["jobId"],
                {
                    "captureManifest": request["captureManifest"],
                    "chunkCount": 1,
                },
            )
            worker_payload = {
                "model": {"id": "private-asr", "revision": "revision-1"},
                "transcript": {"text": "Crash-safe private transcript."},
            }

            from yap_server.jobs import job_store as store_module

            original_publish_json = store_module.publish_json

            def fail_final_state(path: Path, value: object) -> None:
                if path.name == "state.json" and (
                    path.parent / "result-revision.json"
                ).exists():
                    raise OSError("C:/private/recordings/patient-audio.wav")
                original_publish_json(path, value)

            with self.assertNoLogs("concurrent.futures", level="ERROR"):
                with patch.object(store_module, "publish_json", fail_final_state):
                    processor.future.set_result(worker_payload)

            self.assertEqual(service.get(created["jobId"])["status"], "complete")
            self.assertEqual(
                service.get_result(created["jobId"])["transcript"],
                "Crash-safe private transcript.",
            )

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:04:00Z",
                startup_worker_cleanup_verified=True,
            )
            self.assertEqual(restarted.get(created["jobId"])["status"], "complete")
            self.assertEqual(
                restarted.get_result(created["jobId"])["transcript"],
                "Crash-safe private transcript.",
            )

    def test_chunk_replay_is_idempotent_and_conflicting_content_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:04:00Z",
            )
            created = service.create(_create_request())
            chunk = bytes(320)
            arguments = {
                "track_id": "track-1",
                "sequence_start": 0,
                "sequence_end": 159,
                "idempotency_key": "1/s-phase5-create/track-1/0/159",
                "content_sha256": hashlib.sha256(chunk).hexdigest(),
                "audio_codec": "pcm_s16le",
                "sample_rate_hz": 16000,
                "channels": 1,
                "content_length": len(chunk),
            }
            first = service.accept_chunk(
                service.prepare_chunk_upload(created["jobId"], **arguments),
                chunk,
            )
            replayed = service.accept_chunk(
                service.prepare_chunk_upload(created["jobId"], **arguments),
                chunk,
            )

            self.assertEqual(first["disposition"], "accepted")
            self.assertEqual(replayed["disposition"], "replayed")
            self.assertEqual(replayed["acceptedAtUtc"], first["acceptedAtUtc"])

            with self.assertRaises(JobServiceError) as conflict:
                service.prepare_chunk_upload(
                    created["jobId"],
                    **{**arguments, "content_sha256": "c" * 64},
                )
            self.assertEqual(conflict.exception.status, 409)
            self.assertEqual(conflict.exception.code, "CONTENT_IDENTITY_CONFLICT")

    def test_chunk_replay_heals_a_failed_receipt_state_write_before_acknowledging(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:04:00Z",
            )
            created = service.create(_create_request())
            chunk = bytes(320)
            arguments = {
                "track_id": "track-1",
                "sequence_start": 0,
                "sequence_end": 159,
                "idempotency_key": "1/s-phase5-create/track-1/0/159",
                "content_sha256": hashlib.sha256(chunk).hexdigest(),
                "audio_codec": "pcm_s16le",
                "sample_rate_hz": 16000,
                "channels": 1,
                "content_length": len(chunk),
            }

            from yap_server.jobs import job_store as store_module

            original_publish_json = store_module.publish_json

            def fail_uploading_state(path: Path, value: object) -> None:
                if path.name == "state.json" and value["projection"]["status"] == "uploading":
                    raise OSError("private receipt storage unavailable")
                original_publish_json(path, value)

            with patch.object(store_module, "publish_json", fail_uploading_state):
                with self.assertRaises(OSError):
                    service.accept_chunk(
                        service.prepare_chunk_upload(created["jobId"], **arguments),
                        chunk,
                    )

            replayed = service.accept_chunk(
                service.prepare_chunk_upload(created["jobId"], **arguments),
                chunk,
            )
            self.assertEqual(replayed["disposition"], "replayed")

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:05:00Z",
            )
            restarted_replay = restarted.accept_chunk(
                restarted.prepare_chunk_upload(created["jobId"], **arguments),
                chunk,
            )
            self.assertEqual(restarted_replay["disposition"], "replayed")

    def test_restart_rejects_a_receipt_not_bound_to_the_validated_declaration(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:04:00Z",
            )
            created = service.create(_create_request())
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            state_path = root / "jobs" / created["jobId"] / "state.json"
            state = json.loads(state_path.read_text(encoding="utf-8"))
            state["receipts"][0]["replayKey"]["trackId"] = "../../outside"
            state_path.write_text(
                json.dumps(state, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )

            with self.assertRaises(ValueError):
                RecordingJobService(
                    root,
                    processor=_Processor(),
                    supported_languages=("en",),
                    now=lambda: "2026-07-14T21:05:00Z",
                )

    def test_restart_rejects_a_projection_not_bound_to_validated_creation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:04:00Z",
            )
            created = service.create(_create_request())
            state_path = root / "jobs" / created["jobId"] / "state.json"
            state = json.loads(state_path.read_text(encoding="utf-8"))
            state["projection"]["displayName"] = "tampered private title"
            state_path.write_text(
                json.dumps(state, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(
                ValueError,
                "persisted job projection differs from creation",
            ):
                RecordingJobService(
                    root,
                    processor=_Processor(),
                    supported_languages=("en",),
                    now=lambda: "2026-07-14T21:05:00Z",
                )

    def test_create_rejects_path_escaping_track_identity_before_disk_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:05:00Z",
            )
            request = _create_request()
            request["tracks"][0]["trackId"] = "../escape"
            request["chunks"][0]["replayKey"]["trackId"] = "../escape"

            with self.assertRaises(JobServiceError) as invalid:
                service.create(request)

            self.assertEqual(invalid.exception.status, 400)
            self.assertEqual(invalid.exception.code, "INVALID_JOB")
            self.assertFalse((root / "jobs").exists())

    def test_worker_failure_becomes_retryable_job_failure_without_a_result(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            processor = _ControlledProcessor()
            service = RecordingJobService(
                Path(temporary),
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:06:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            plan = service.prepare_chunk_upload(
                created["jobId"],
                track_id="track-1",
                sequence_start=0,
                sequence_end=159,
                idempotency_key="1/s-phase5-create/track-1/0/159",
                content_sha256=hashlib.sha256(chunk).hexdigest(),
                audio_codec="pcm_s16le",
                sample_rate_hz=16000,
                channels=1,
                content_length=len(chunk),
            )
            service.accept_chunk(plan, chunk)
            service.commit(
                created["jobId"],
                {"captureManifest": request["captureManifest"], "chunkCount": 1},
            )

            processor.future.set_exception(RuntimeError("private worker details"))

            failed = service.get(created["jobId"])
            self.assertEqual(failed["status"], "failed")
            self.assertEqual(
                failed["error"],
                {
                    "code": "ASR_WORKER_FAILED",
                    "message": "The private ASR worker did not complete the job.",
                    "retryable": True,
                    "requestId": f"job-{created['jobId']}",
                },
            )
            with self.assertRaises(JobServiceError) as missing:
                service.get_result(created["jobId"])
            self.assertEqual(missing.exception.status, 409)
            self.assertEqual(missing.exception.code, "RESULT_NOT_READY")
            job_root = Path(temporary) / "jobs" / created["jobId"]
            self.assertEqual(list((job_root / "chunks").iterdir()), [])
            self.assertFalse((job_root / "input.wav").exists())
            restarted = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:06:01Z",
            )
            self.assertEqual(restarted.get(created["jobId"]), failed)

    def test_cancellation_is_idempotent_before_worker_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:00Z",
            )
            created = service.create(_create_request())

            cancelled = service.cancel(created["jobId"])
            replayed = service.cancel(created["jobId"])

            self.assertEqual(cancelled["status"], "cancelled")
            self.assertEqual(replayed, cancelled)
            with self.assertRaises(JobServiceError) as blocked:
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(bytes(320)).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=320,
                )
            self.assertEqual(blocked.exception.status, 409)
            self.assertEqual(blocked.exception.code, "JOB_NOT_UPLOADABLE")

    def test_running_cancellation_waits_for_worker_cleanup_before_acknowledging(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worker = _ActiveCancellationWorker()
            pool = BatchAsrPool(worker, max_workers=1, max_queued=0)
            try:
                service = RecordingJobService(
                    root,
                    processor=pool,
                    supported_languages=("en",),
                    now=lambda: "2026-07-14T21:07:15Z",
                )
                request = _create_request()
                created = service.create(request)
                chunk = bytes(320)
                service.accept_chunk(
                    service.prepare_chunk_upload(
                        created["jobId"],
                        track_id="track-1",
                        sequence_start=0,
                        sequence_end=159,
                        idempotency_key="1/s-phase5-create/track-1/0/159",
                        content_sha256=hashlib.sha256(chunk).hexdigest(),
                        audio_codec="pcm_s16le",
                        sample_rate_hz=16000,
                        channels=1,
                        content_length=len(chunk),
                    ),
                    chunk,
                )
                service.commit(
                    created["jobId"],
                    {
                        "captureManifest": request["captureManifest"],
                        "chunkCount": 1,
                    },
                )
                self.assertTrue(worker.started.wait(timeout=2))

                cancelled = service.cancel(created["jobId"])

                self.assertTrue(worker.stopped.is_set())
                self.assertEqual(cancelled["status"], "cancelled")
                self.assertEqual(pool.outstanding_count, 0)
                job_root = root / "jobs" / created["jobId"]
                self.assertEqual(list((job_root / "chunks").iterdir()), [])
                self.assertFalse((job_root / "input.wav").exists())
            finally:
                pool.shutdown()

    def test_unverified_cleanup_fails_cancellation_and_fences_capacity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            worker = _UnverifiedCleanupWorker()
            pool = BatchAsrPool(worker, max_workers=1, max_queued=0)
            try:
                service = RecordingJobService(
                    root,
                    processor=pool,
                    supported_languages=("en",),
                    now=lambda: "2026-07-14T21:07:20Z",
                )
                request = _create_request()
                created = service.create(request)
                chunk = bytes(320)
                service.accept_chunk(
                    service.prepare_chunk_upload(
                        created["jobId"],
                        track_id="track-1",
                        sequence_start=0,
                        sequence_end=159,
                        idempotency_key="1/s-phase5-create/track-1/0/159",
                        content_sha256=hashlib.sha256(chunk).hexdigest(),
                        audio_codec="pcm_s16le",
                        sample_rate_hz=16000,
                        channels=1,
                        content_length=len(chunk),
                    ),
                    chunk,
                )
                service.commit(
                    created["jobId"],
                    {
                        "captureManifest": request["captureManifest"],
                        "chunkCount": 1,
                    },
                )
                self.assertTrue(worker.started.wait(timeout=2))

                with self.assertRaises(JobServiceError) as cancellation:
                    service.cancel(created["jobId"])

                self.assertEqual(cancellation.exception.status, 503)
                self.assertEqual(
                    cancellation.exception.code,
                    "CANCELLATION_CLEANUP_UNVERIFIED",
                )
                self.assertTrue(cancellation.exception.retryable)
                failed = service.get(created["jobId"])
                self.assertEqual(failed["status"], "failed")
                self.assertEqual(failed["error"]["code"], "ASR_CLEANUP_UNVERIFIED")
                self.assertTrue(pool.fenced)
                self.assertEqual(pool.outstanding_count, 0)
                state = json.loads(
                    (root / "jobs" / created["jobId"] / "state.json").read_text(
                        encoding="utf-8"
                    )
                )
                self.assertTrue(state["cancellationRequested"])

                with self.assertRaisesRegex(ValueError, "startup cleanup"):
                    RecordingJobService(
                        root,
                        processor=_Processor(),
                        supported_languages=("en",),
                        now=lambda: "2026-07-14T21:07:21Z",
                    )
                restarted = RecordingJobService(
                    root,
                    processor=_Processor(),
                    supported_languages=("en",),
                    now=lambda: "2026-07-14T21:07:21Z",
                    startup_worker_cleanup_verified=True,
                )
                self.assertEqual(restarted.get(created["jobId"])["status"], "cancelled")
            finally:
                pool.shutdown()

    def test_pending_cancellation_survives_restart_and_converges_to_cancelled(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            processor = _UnstoppableProcessor()
            service = RecordingJobService(
                root,
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:25Z",
                cancellation_timeout_seconds=0.01,
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            service.commit(
                created["jobId"],
                {
                    "captureManifest": request["captureManifest"],
                    "chunkCount": 1,
                },
            )

            with self.assertRaises(JobServiceError) as pending:
                service.cancel(created["jobId"])
            self.assertEqual(pending.exception.code, "CANCELLATION_PENDING")

            with self.assertRaisesRegex(ValueError, "startup cleanup"):
                RecordingJobService(
                    root,
                    processor=_Processor(),
                    supported_languages=("en",),
                    now=lambda: "2026-07-14T21:07:26Z",
                )
            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:26Z",
                startup_worker_cleanup_verified=True,
            )
            self.assertEqual(restarted.get(created["jobId"])["status"], "cancelled")
            job_root = root / "jobs" / created["jobId"]
            self.assertEqual(list((job_root / "chunks").iterdir()), [])
            self.assertFalse((job_root / "input.wav").exists())

            processor.future.set_exception(WorkerExecutionError("test cleanup"))

    def test_cancellation_purges_uploaded_audio_but_keeps_a_restart_safe_tombstone(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:30Z",
            )
            created = service.create(_create_request(), idempotency_key="cancel-purge")
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            job_root = root / "jobs" / created["jobId"]
            self.assertTrue(any((job_root / "chunks").iterdir()))

            cancelled = service.cancel(created["jobId"])

            self.assertEqual(cancelled["status"], "cancelled")
            self.assertEqual(list((job_root / "chunks").iterdir()), [])
            persisted = json.loads((job_root / "state.json").read_text(encoding="utf-8"))
            self.assertEqual(persisted["receipts"], [])
            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:31Z",
                startup_worker_cleanup_verified=True,
            )
            self.assertEqual(restarted.get(created["jobId"]), cancelled)

    def test_cancel_retry_heals_a_failed_tombstone_write_before_acknowledging(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:30Z",
            )
            created = service.create(_create_request())
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            job_root = root / "jobs" / created["jobId"]

            from yap_server.jobs import job_store as store_module

            original_publish_json = store_module.publish_json

            def fail_cancelled_state(path: Path, value: object) -> None:
                if path.name == "state.json" and value["projection"]["status"] == "cancelled":
                    raise OSError("private cancellation storage unavailable")
                original_publish_json(path, value)

            with patch.object(store_module, "publish_json", fail_cancelled_state):
                with self.assertRaises(OSError):
                    service.cancel(created["jobId"])

            self.assertTrue(any((job_root / "chunks").iterdir()))
            cancelled = service.cancel(created["jobId"])
            self.assertEqual(cancelled["status"], "cancelled")
            self.assertEqual(list((job_root / "chunks").iterdir()), [])

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:31Z",
                startup_worker_cleanup_verified=True,
            )
            self.assertEqual(restarted.get(created["jobId"])["status"], "cancelled")

    def test_cancellation_between_chunk_plan_and_body_acceptance_cannot_resurrect_the_job(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:07:45Z",
            )
            created = service.create(_create_request())
            chunk = bytes(320)
            plan = service.prepare_chunk_upload(
                created["jobId"],
                track_id="track-1",
                sequence_start=0,
                sequence_end=159,
                idempotency_key="1/s-phase5-create/track-1/0/159",
                content_sha256=hashlib.sha256(chunk).hexdigest(),
                audio_codec="pcm_s16le",
                sample_rate_hz=16000,
                channels=1,
                content_length=len(chunk),
            )
            service.cancel(created["jobId"])

            with self.assertRaises(JobServiceError) as blocked:
                service.accept_chunk(plan, chunk)

            self.assertEqual(blocked.exception.status, 409)
            self.assertEqual(blocked.exception.code, "JOB_NOT_UPLOADABLE")
            self.assertEqual(service.get(created["jobId"])["status"], "cancelled")
            self.assertEqual(
                list((root / "jobs" / created["jobId"] / "chunks").iterdir()),
                [],
            )

    def test_restart_restores_job_and_chunk_replay_without_reaccepting_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:08:00Z",
            )
            created = service.create(_create_request())
            chunk = bytes(320)
            arguments = {
                "track_id": "track-1",
                "sequence_start": 0,
                "sequence_end": 159,
                "idempotency_key": "1/s-phase5-create/track-1/0/159",
                "content_sha256": hashlib.sha256(chunk).hexdigest(),
                "audio_codec": "pcm_s16le",
                "sample_rate_hz": 16000,
                "channels": 1,
                "content_length": len(chunk),
            }
            accepted = service.accept_chunk(
                service.prepare_chunk_upload(created["jobId"], **arguments),
                chunk,
            )

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:09:00Z",
            )
            replayed = restarted.accept_chunk(
                restarted.prepare_chunk_upload(created["jobId"], **arguments),
                chunk,
            )

            self.assertEqual(restarted.get(created["jobId"])["status"], "uploading")
            self.assertEqual(replayed["disposition"], "replayed")
            self.assertEqual(replayed["acceptedAtUtc"], accepted["acceptedAtUtc"])

    def test_restart_promotes_an_atomically_published_result_to_complete(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:19:00Z",
            )
            created = service.create(_create_request())
            job_root = root / "jobs" / created["jobId"]
            result = _published_result(created)
            (job_root / "result-revision.json").write_text(
                json.dumps(result, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )
            state_path = job_root / "state.json"
            state = json.loads(state_path.read_text(encoding="utf-8"))
            state["projection"]["status"] = "server_processing"
            state_path.write_text(
                json.dumps(state, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:21:00Z",
                startup_worker_cleanup_verified=True,
            )

            self.assertEqual(restarted.get(created["jobId"])["status"], "complete")
            self.assertEqual(restarted.get_result(created["jobId"]), result)
            persisted = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertEqual(persisted["projection"]["status"], "complete")

    def test_restart_discards_an_orphan_result_for_a_cancelled_job(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:22:00Z",
            )
            created = service.create(_create_request())
            cancelled = service.cancel(created["jobId"])
            result_path = root / "jobs" / created["jobId"] / "result-revision.json"
            result_path.write_text(
                json.dumps(_published_result(created), separators=(",", ":")) + "\n",
                encoding="utf-8",
            )

            restarted = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:23:00Z",
                startup_worker_cleanup_verified=True,
            )

            self.assertEqual(restarted.get(created["jobId"]), cancelled)
            self.assertFalse(result_path.exists())
            with self.assertRaises(JobServiceError) as unavailable:
                restarted.get_result(created["jobId"])
            self.assertEqual(unavailable.exception.code, "RESULT_NOT_READY")

    def test_commit_backpressure_is_retryable_without_losing_uploaded_state(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_BusyProcessor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:11:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )

            with self.assertRaises(JobServiceError) as busy:
                service.commit(
                    created["jobId"],
                    {
                        "captureManifest": request["captureManifest"],
                        "chunkCount": 1,
                    },
                )

            self.assertEqual(busy.exception.status, 429)
            self.assertEqual(busy.exception.code, "SERVER_BUSY")
            self.assertTrue(busy.exception.retryable)
            self.assertEqual(service.get(created["jobId"])["status"], "uploading")

    def test_cancellation_during_commit_cannot_dispatch_or_restore_processing(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            processor = _ControlledProcessor()
            service = RecordingJobService(
                Path(temporary),
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:12:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            entered_publish = threading.Event()
            release_publish = threading.Event()
            outcome: dict[str, object] = {}

            from yap_server.jobs import service as service_module

            original_publish_wav = service_module._publish_wav

            def blocked_publish_wav(path: Path, chunks: list[Path]) -> None:
                entered_publish.set()
                if not release_publish.wait(timeout=2):
                    raise TimeoutError("test WAV publication was not released")
                original_publish_wav(path, chunks)

            def commit() -> None:
                try:
                    outcome["projection"] = service.commit(
                        created["jobId"],
                        {
                            "captureManifest": request["captureManifest"],
                            "chunkCount": 1,
                        },
                    )
                except Exception as error:  # pragma: no cover - assertion below reports it
                    outcome["error"] = error

            def cancel() -> None:
                try:
                    outcome["cancelled"] = service.cancel(created["jobId"])
                except Exception as error:  # pragma: no cover - assertion below reports it
                    outcome["cancel_error"] = error

            with patch.object(service_module, "_publish_wav", blocked_publish_wav):
                committing = threading.Thread(target=commit)
                committing.start()
                self.assertTrue(entered_publish.wait(timeout=2))
                cancelling = threading.Thread(target=cancel)
                cancelling.start()
                self.assertTrue(cancelling.is_alive())
                release_publish.set()
                committing.join(timeout=2)
                cancelling.join(timeout=2)

            self.assertFalse(committing.is_alive())
            self.assertFalse(cancelling.is_alive())
            self.assertNotIn("error", outcome)
            self.assertNotIn("cancel_error", outcome)
            self.assertEqual(outcome["projection"], outcome["cancelled"])
            self.assertEqual(service.get(created["jobId"])["status"], "cancelled")
            self.assertEqual(processor.jobs, [])
            job_root = Path(temporary) / "jobs" / created["jobId"]
            self.assertFalse((job_root / "input.wav").exists())
            self.assertEqual(list((job_root / "chunks").iterdir()), [])

    def test_cancellation_during_failed_commit_still_purges_private_audio(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            processor = _ControlledProcessor()
            service = RecordingJobService(
                Path(temporary),
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:12:30Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            entered_publish = threading.Event()
            release_publish = threading.Event()
            outcome: dict[str, object] = {}

            from yap_server.jobs import service as service_module

            def failing_publish_wav(_path: Path, _chunks: list[Path]) -> None:
                entered_publish.set()
                if not release_publish.wait(timeout=2):
                    raise TimeoutError("test WAV publication was not released")
                raise OSError("injected WAV publication failure")

            def commit() -> None:
                try:
                    service.commit(
                        created["jobId"],
                        {
                            "captureManifest": request["captureManifest"],
                            "chunkCount": 1,
                        },
                    )
                except Exception as error:  # pragma: no cover - asserted below
                    outcome["error"] = error

            def cancel() -> None:
                try:
                    outcome["cancelled"] = service.cancel(created["jobId"])
                except Exception as error:  # pragma: no cover - asserted below
                    outcome["cancel_error"] = error

            with patch.object(service_module, "_publish_wav", failing_publish_wav):
                committing = threading.Thread(target=commit)
                committing.start()
                self.assertTrue(entered_publish.wait(timeout=2))
                cancelling = threading.Thread(target=cancel)
                cancelling.start()
                self.assertTrue(cancelling.is_alive())
                release_publish.set()
                committing.join(timeout=2)
                cancelling.join(timeout=2)

            self.assertFalse(committing.is_alive())
            self.assertFalse(cancelling.is_alive())
            self.assertNotIn("cancel_error", outcome)
            self.assertEqual(outcome["cancelled"]["status"], "cancelled")
            self.assertIsInstance(outcome.get("error"), OSError)
            self.assertEqual(
                str(outcome["error"]),
                "injected WAV publication failure",
            )
            self.assertEqual(service.get(created["jobId"])["status"], "cancelled")
            self.assertEqual(processor.jobs, [])
            job_root = Path(temporary) / "jobs" / created["jobId"]
            self.assertFalse((job_root / "input.wav").exists())
            self.assertEqual(list((job_root / "chunks").iterdir()), [])

    def test_cancelled_result_publication_removes_the_uncommitted_result(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            processor = _ControlledProcessor()
            service = RecordingJobService(
                root,
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:13:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            service.commit(
                created["jobId"],
                {"captureManifest": request["captureManifest"], "chunkCount": 1},
            )
            worker_job = processor.jobs[0]
            payload = {
                "transcript": {"text": "Cancelled private transcript."},
                "model": {"id": "private-asr", "revision": "revision-1"},
            }
            entered_publish = threading.Event()
            release_publish = threading.Event()
            cancellation: dict[str, object] = {}

            from yap_server.jobs import completion as completion_module

            original_publish_json = completion_module.publish_json

            def blocked_publish_json(path: Path, value: object) -> None:
                if path.name == "result-revision.json":
                    entered_publish.set()
                    if not release_publish.wait(timeout=2):
                        raise TimeoutError("test result publication was not released")
                original_publish_json(path, value)

            def cancel() -> None:
                try:
                    cancellation["projection"] = service.cancel(created["jobId"])
                except Exception as error:  # pragma: no cover - asserted below
                    cancellation["error"] = error

            with patch.object(completion_module, "publish_json", blocked_publish_json):
                completing = threading.Thread(
                    target=processor.future.set_result,
                    args=(payload,),
                )
                completing.start()
                self.assertTrue(entered_publish.wait(timeout=2))
                cancelling = threading.Thread(target=cancel)
                cancelling.start()
                self.assertTrue(cancelling.is_alive())
                release_publish.set()
                completing.join(timeout=2)
                cancelling.join(timeout=2)

            self.assertFalse(completing.is_alive())
            self.assertFalse(cancelling.is_alive())
            self.assertNotIn("error", cancellation)
            cancelled = cancellation["projection"]
            self.assertEqual(cancelled["status"], "cancelled")
            self.assertEqual(service.get(created["jobId"])["status"], "cancelled")
            self.assertFalse(
                (root / "jobs" / created["jobId"] / "result-revision.json").exists()
            )

    def test_invalid_worker_result_becomes_a_safe_retryable_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            processor = _ControlledProcessor()
            service = RecordingJobService(
                Path(temporary),
                processor=processor,
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:14:00Z",
            )
            request = _create_request()
            created = service.create(request)
            chunk = bytes(320)
            service.accept_chunk(
                service.prepare_chunk_upload(
                    created["jobId"],
                    track_id="track-1",
                    sequence_start=0,
                    sequence_end=159,
                    idempotency_key="1/s-phase5-create/track-1/0/159",
                    content_sha256=hashlib.sha256(chunk).hexdigest(),
                    audio_codec="pcm_s16le",
                    sample_rate_hz=16000,
                    channels=1,
                    content_length=len(chunk),
                ),
                chunk,
            )
            service.commit(
                created["jobId"],
                {"captureManifest": request["captureManifest"], "chunkCount": 1},
            )
            processor.future.set_result({"transcript": {"text": "missing model"}})

            failed = service.get(created["jobId"])
            self.assertEqual(failed["status"], "failed")
            self.assertEqual(failed["error"]["code"], "ASR_RESULT_INVALID")
            self.assertTrue(failed["error"]["retryable"])

    def test_expired_terminal_jobs_are_pruned_before_new_intake(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clock = {"now": "2026-07-14T21:15:00Z"}
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: clock["now"],
            )
            expired = service.create(
                _create_request(retention_expires_at_utc="2026-07-15T00:00:00Z"),
                idempotency_key="expired-create",
            )
            service.cancel(expired["jobId"])
            expired_root = root / "jobs" / expired["jobId"]
            self.assertTrue(expired_root.is_dir())
            clock["now"] = "2026-07-16T00:00:00Z"

            fresh = service.create(
                _create_request(
                    session_id="s-phase5-fresh",
                    retention_expires_at_utc="2026-08-13T21:00:00Z",
                ),
                idempotency_key="fresh-create",
            )

            self.assertEqual(fresh["status"], "accepted")
            self.assertFalse(expired_root.exists())
            with self.assertRaises(KeyError):
                service.get(expired["jobId"])

    def test_idle_maintenance_prunes_expired_terminal_jobs_without_new_intake(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clock = {"now": "2026-07-14T21:15:00Z"}
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: clock["now"],
            )
            expired = service.create(
                _create_request(retention_expires_at_utc="2026-07-15T00:00:00Z")
            )
            service.cancel(expired["jobId"])
            expired_root = root / "jobs" / expired["jobId"]
            clock["now"] = "2026-07-16T00:00:00Z"

            self.assertEqual(service.prune_expired(), 1)
            self.assertFalse(expired_root.exists())
            with self.assertRaises(KeyError):
                service.get(expired["jobId"])

    def test_idle_maintenance_cancels_and_removes_expired_uncommitted_audio(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clock = {"now": "2026-07-14T21:15:00Z"}
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: clock["now"],
            )
            expired = service.create(
                _create_request(retention_expires_at_utc="2026-07-15T00:00:00Z")
            )
            expired_root = root / "jobs" / expired["jobId"]
            clock["now"] = "2026-07-16T00:00:00Z"

            self.assertEqual(service.prune_expired(), 1)
            self.assertFalse(expired_root.exists())
            with self.assertRaises(KeyError):
                service.get(expired["jobId"])

    def test_expired_running_job_stays_nonterminal_until_worker_cleanup_finishes(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clock = {"now": "2026-07-14T21:15:00Z"}
            worker = _DelayedCancellationWorker()
            pool = BatchAsrPool(worker, max_workers=1, max_queued=0)
            try:
                service = RecordingJobService(
                    root,
                    processor=pool,
                    supported_languages=("en",),
                    now=lambda: clock["now"],
                )
                request = _create_request(
                    retention_expires_at_utc="2026-07-15T00:00:00Z"
                )
                created = service.create(request)
                chunk = bytes(320)
                service.accept_chunk(
                    service.prepare_chunk_upload(
                        created["jobId"],
                        track_id="track-1",
                        sequence_start=0,
                        sequence_end=159,
                        idempotency_key="1/s-phase5-create/track-1/0/159",
                        content_sha256=hashlib.sha256(chunk).hexdigest(),
                        audio_codec="pcm_s16le",
                        sample_rate_hz=16000,
                        channels=1,
                        content_length=len(chunk),
                    ),
                    chunk,
                )
                service.commit(
                    created["jobId"],
                    {
                        "captureManifest": request["captureManifest"],
                        "chunkCount": 1,
                    },
                )
                self.assertTrue(worker.started.wait(timeout=2))
                job_root = root / "jobs" / created["jobId"]
                clock["now"] = "2026-07-16T00:00:00Z"

                self.assertEqual(service.prune_expired(), 0)
                self.assertTrue(worker.cancellation_received.wait(timeout=2))
                self.assertEqual(
                    service.get(created["jobId"])["status"],
                    "server_processing",
                )
                self.assertTrue(job_root.is_dir())

                worker.release_cleanup.set()
                deadline = time.monotonic() + 2
                while pool.outstanding_count and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertEqual(pool.outstanding_count, 0)
                self.assertEqual(service.prune_expired(), 1)
                self.assertFalse(job_root.exists())
            finally:
                worker.release_cleanup.set()
                pool.shutdown()

    def test_active_job_count_cap_fails_closed_without_mutating_storage(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = RecordingJobService(
                root,
                processor=_Processor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:16:00Z",
            )

            with patch("yap_server.jobs.service._MAX_STORED_JOBS", 1):
                first = service.create(_create_request())
                with self.assertRaises(JobServiceError) as full:
                    service.create(_create_request(session_id="s-phase5-second"))

            self.assertEqual(full.exception.status, 429)
            self.assertEqual(full.exception.code, "SERVER_STORAGE_LIMIT")
            self.assertFalse(full.exception.retryable)
            self.assertEqual(
                [path.name for path in (root / "jobs").iterdir()],
                [first["jobId"]],
            )


if __name__ == "__main__":
    unittest.main()
