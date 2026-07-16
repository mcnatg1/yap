from __future__ import annotations

import hashlib
import json
import tempfile
import time
import unittest
from pathlib import Path

from yap_server.jobs import JobServiceError, RecordingJobService
from yap_server.pools.batch_asr import BatchAsrPool, WorkerExecutionError

from .service_fixtures import (
    _ActiveCancellationWorker,
    _Processor,
    _UnstoppableProcessor,
    _UnverifiedCleanupWorker,
    _create_request,
)


class RecordingJobCancellationTests(unittest.TestCase):
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
