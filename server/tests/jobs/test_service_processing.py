from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import wave

from yap_server.jobs import JobServiceError, RecordingJobService

from .service_fixtures import (
    _BusyProcessor,
    _ControlledProcessor,
    _Processor,
    _create_request,
)


class RecordingJobProcessingTests(unittest.TestCase):
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
