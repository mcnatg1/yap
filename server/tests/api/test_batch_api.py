import hashlib
import io
import json
import socket
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

from .api_fixtures import BatchJobApiTestCase, _phase5_job_request


class BatchJobApiTests(BatchJobApiTestCase):
    def test_batch_create_requires_one_idempotency_key(self) -> None:
        status, _, payload = self._request(
            "/v1/jobs",
            method="POST",
            headers={"Content-Type": "application/json"},
            data=json.dumps(_phase5_job_request()).encode("utf-8"),
        )

        self.assertEqual(status, 400)
        self.assertEqual(payload["code"], "IDEMPOTENCY_KEY_REQUIRED")

    def test_pre_body_rejection_drains_the_bounded_body_before_responding(self) -> None:
        host, port = self.server.server_address[:2]
        with socket.create_connection((host, port), timeout=2) as client:
            client.settimeout(0.1)
            client.sendall(
                b"POST /v1/jobs HTTP/1.1\r\n"
                b"Host: localhost\r\n"
                b"Content-Type: application/json\r\n"
                b"Content-Length: 2\r\n"
                b"\r\n"
            )
            with self.assertRaises(socket.timeout):
                client.recv(1)

            client.sendall(b"{}")
            client.settimeout(2)
            response = bytearray()
            while True:
                block = client.recv(4096)
                if not block:
                    break
                response.extend(block)

        head, body = bytes(response).split(b"\r\n\r\n", 1)
        self.assertIn(b" 400 ", head)
        self.assertEqual(json.loads(body)["code"], "IDEMPOTENCY_KEY_REQUIRED")

    def test_storage_failures_return_a_generic_error_without_private_paths(self) -> None:
        private_path = "C:/private/recordings/patient-audio.wav"
        stderr = io.StringIO()

        with redirect_stderr(stderr):
            with patch.object(
                self.jobs,
                "create",
                side_effect=OSError(f"could not write {private_path}"),
            ):
                status, _, payload = self._request(
                    "/v1/jobs",
                    method="POST",
                    headers={
                        "Content-Type": "application/json",
                        "Idempotency-Key": "job-api-storage-error",
                    },
                    data=json.dumps(_phase5_job_request()).encode("utf-8"),
                )

        self.assertEqual(status, 500)
        self.assertEqual(payload["code"], "SERVER_STORAGE_ERROR")
        self.assertTrue(payload["retryable"])
        self.assertEqual(
            payload["message"],
            "Private recording storage could not complete the request.",
        )
        observable_output = "\n".join(
            [stderr.getvalue(), json.dumps(payload), *self.logger.messages]
        )
        self.assertNotIn(private_path, observable_output)

    def test_batch_contract_runs_create_upload_commit_status_and_result(self) -> None:
        job_request = _phase5_job_request()
        status, _, health_payload = self._request("/v1/health")
        self.assertEqual(status, 200)
        self.assertEqual(
            health_payload["capabilities"],
            {"batchJobs": True, "liveStreaming": False, "jobStatus": True},
        )

        status, _, created = self._request(
            "/v1/jobs",
            method="POST",
            headers={
                "Content-Type": "application/json",
                "Idempotency-Key": "job-api-create",
            },
            data=json.dumps(job_request).encode("utf-8"),
        )
        self.assertEqual(status, 202)
        job_id = created["jobId"]

        chunk = bytes(320)
        digest = hashlib.sha256(chunk).hexdigest()
        status, _, receipt = self._request(
            f"/v1/jobs/{job_id}/chunks/track-1/0-159",
            method="PUT",
            headers={
                "Content-Type": "application/octet-stream",
                "Idempotency-Key": "1/s-phase5-api/track-1/0/159",
                "X-Yap-Content-SHA256": digest,
                "X-Yap-Audio-Codec": "pcm_s16le",
                "X-Yap-Sample-Rate-Hz": "16000",
                "X-Yap-Channels": "1",
            },
            data=chunk,
        )
        self.assertEqual(status, 201)
        self.assertEqual(receipt["disposition"], "accepted")

        status, _, committed = self._request(
            f"/v1/jobs/{job_id}/commit",
            method="POST",
            headers={
                "Content-Type": "application/json",
                "Idempotency-Key": "job-api-cancel",
            },
            data=json.dumps(
                {
                    "captureManifest": job_request["captureManifest"],
                    "chunkCount": 1,
                }
            ).encode("utf-8"),
        )
        self.assertEqual(status, 202)
        self.assertEqual(committed["status"], "server_processing")
        worker_job = self.processor.jobs[0]
        self.processor.future.set_result(
            {
                "schemaVersion": 1,
                "jobId": job_id,
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
                    "text": "Private transcript must not enter request logs.",
                    "language": "en",
                    "punctuation": True,
                },
            }
        )

        status, _, completed = self._request(f"/v1/jobs/{job_id}")
        self.assertEqual(status, 200)
        self.assertEqual(completed["status"], "complete")
        status, _, result = self._request(f"/v1/jobs/{job_id}/result")
        self.assertEqual(status, 200)
        self.assertEqual(
            result["transcript"],
            "Private transcript must not enter request logs.",
        )
        self.assertNotIn(
            result["transcript"],
            "\n".join(self.logger.messages),
        )

        job_root = Path(self.temporary.name) / "jobs" / job_id
        self.assertTrue((job_root / "input.wav").is_file())
        self.assertTrue((job_root / "result-revision.json").is_file())
        status, _, cancelled = self._request(
            f"/v1/jobs/{job_id}",
            method="DELETE",
        )
        self.assertEqual(status, 202)
        self.assertEqual(cancelled["status"], "cancelled")
        result_status, _, missing_result = self._request(
            f"/v1/jobs/{job_id}/result"
        )
        self.assertEqual(result_status, 409)
        self.assertEqual(missing_result["code"], "RESULT_NOT_READY")
        self.assertEqual(list((job_root / "chunks").iterdir()), [])
        self.assertFalse((job_root / "input.wav").exists())
        self.assertFalse((job_root / "result-revision.json").exists())

    def test_batch_cancellation_route_records_and_replays_terminal_state(self) -> None:
        status, _, created = self._request(
            "/v1/jobs",
            method="POST",
            headers={
                "Content-Type": "application/json",
                "Idempotency-Key": "job-api-cancel",
            },
            data=json.dumps(_phase5_job_request()).encode("utf-8"),
        )
        self.assertEqual(status, 202)

        status, _, cancelled = self._request(
            f"/v1/jobs/{created['jobId']}",
            method="DELETE",
        )
        replay_status, _, replayed = self._request(
            f"/v1/jobs/{created['jobId']}",
            method="DELETE",
        )

        self.assertEqual(status, 202)
        self.assertEqual(replay_status, 202)
        self.assertEqual(cancelled["status"], "cancelled")
        self.assertEqual(replayed, cancelled)
