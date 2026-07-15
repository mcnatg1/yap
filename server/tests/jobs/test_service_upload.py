from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from yap_server.jobs import JobServiceError, RecordingJobService

from .service_fixtures import _Processor, _create_request


class RecordingJobUploadTests(unittest.TestCase):
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
