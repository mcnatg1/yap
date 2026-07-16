from __future__ import annotations

import hashlib
import json
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from yap_server.jobs import JobServiceError, RecordingJobService

from .service_fixtures import (
    _ControlledProcessor,
    _Processor,
    _create_request,
    _published_result,
)


class RecordingJobResultRecoveryTests(unittest.TestCase):
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
