from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from yap_server.jobs import JobServiceError, RecordingJobService
from yap_server.pools.batch_asr_worker import MAX_AUDIO_SECONDS, SAMPLE_RATE_HZ

from .service_fixtures import _Processor, _create_request, _published_result


class RecordingJobContractTests(unittest.TestCase):
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
