from __future__ import annotations

import hashlib
import json
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from yap_server.jobs import JobServiceError, RecordingJobService

from .service_fixtures import _ControlledProcessor, _Processor, _create_request


class RecordingJobCancellationRaceTests(unittest.TestCase):
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
