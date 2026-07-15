from __future__ import annotations

from concurrent.futures import Future
import hashlib
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest
from http.server import HTTPServer, ThreadingHTTPServer
from typing import Any
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from yap_server.api.app import create_server
from yap_server.api.request_io import MAX_REQUEST_BODY_BYTES
from yap_server.config import ServerSettings
from yap_server.jobs import RecordingJobService
from yap_server.pools.batch_asr import BatchAsrJob


class _CapturingLogger:
    def __init__(self) -> None:
        self.messages: list[str] = []

    def info(self, message: str) -> None:
        self.messages.append(message)


class _ControlledProcessor:
    def __init__(self) -> None:
        self.jobs: list[BatchAsrJob] = []
        self.future: Future[dict[str, object]] = Future()

    def submit(self, job: BatchAsrJob) -> Future[dict[str, object]]:
        self.jobs.append(job)
        return self.future


class _BlockingStatusService:
    def __init__(self, saturation: int) -> None:
        self._saturation = saturation
        self._lock = threading.Lock()
        self.release = threading.Event()
        self.saturated = threading.Event()
        self.active = 0
        self.maximum_active = 0

    def get(self, job_id: str) -> dict[str, object]:
        with self._lock:
            self.active += 1
            self.maximum_active = max(self.maximum_active, self.active)
            if self.active >= self._saturation:
                self.saturated.set()
        try:
            if not self.release.wait(timeout=5):
                raise TimeoutError("test request was not released")
            return {"jobId": job_id, "status": "accepted"}
        finally:
            with self._lock:
                self.active -= 1


def _phase5_job_request() -> dict[str, object]:
    chunk = bytes(320)
    return {
        "displayName": "Phase 5 API",
        "metadata": {
            "sessionId": "s-phase5-api",
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
            "retentionExpiresAtUtc": "2026-08-13T21:00:00Z",
        },
        "tracks": [
            {
                "trackId": "track-1",
                "source": {"kind": "imported", "provenance": "unknown"},
                "deviceId": None,
                "originalSampleRateHz": 16000,
                "originalChannels": 1,
            }
        ],
        "route": "server_batch",
        "captureManifest": {
            "schemaVersion": 1,
            "sessionId": "s-phase5-api",
            "sha256": "a" * 64,
            "byteLength": 4096,
        },
        "chunks": [
            {
                "replayKey": {
                    "schemaVersion": 1,
                    "sessionId": "s-phase5-api",
                    "trackId": "track-1",
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


class HealthServerTestCase(unittest.TestCase):
    def setUp(self) -> None:
        self.logger = _CapturingLogger()
        self.server = create_server(
            ServerSettings(host="127.0.0.1", port=0),
            logger=self.logger,
        )
        self.assertIsInstance(self.server, HTTPServer)
        self.assertNotIsInstance(self.server, ThreadingHTTPServer)
        host, port = self.server.server_address[:2]
        self.base_url = f"http://{host}:{port}"
        self.thread = threading.Thread(
            target=self.server.serve_forever,
            kwargs={"poll_interval": 0.01},
            daemon=True,
        )
        self.thread.start()

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.assertFalse(self.thread.is_alive(), "health server did not stop cleanly")

    def _request(
        self,
        path: str,
        *,
        method: str = "GET",
        headers: dict[str, str] | None = None,
        data: bytes | None = None,
        timeout: float = 2,
    ) -> tuple[int, Any, bytes]:
        request = Request(
            f"{self.base_url}{path}",
            data=data,
            headers=headers or {},
            method=method,
        )
        try:
            response = urlopen(request, timeout=timeout)
        except HTTPError as error:
            response = error
        with response:
            body = response.read()
            return response.status, response.headers, body

    def _raw_request(self, request: bytes) -> bytes:
        host, port = self.server.server_address[:2]
        with socket.create_connection((host, port), timeout=2) as client:
            client.sendall(request)
            client.shutdown(socket.SHUT_WR)
            response = bytearray()
            while chunk := client.recv(4096):
                response.extend(chunk)
        return bytes(response)

    def _parse_raw_json_response(
        self, response: bytes
    ) -> tuple[int, dict[str, object]]:
        head, body = response.split(b"\r\n\r\n", 1)
        status = int(head.split(b"\r\n", 1)[0].split()[1])
        return status, json.loads(body)

    def assert_json_headers(self, headers: Any, body: bytes) -> None:
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertEqual(headers["X-Content-Type-Options"], "nosniff")
        self.assertEqual(int(headers["Content-Length"]), len(body))

    def assert_error(
        self,
        status: int,
        headers: Any,
        body: bytes,
        *,
        expected_status: int,
        code: str,
        message: str,
    ) -> dict[str, object]:
        self.assertEqual(status, expected_status)
        self.assert_json_headers(headers, body)
        payload = json.loads(body)
        request_id = payload.get("requestId")
        self.assertIsInstance(request_id, str)
        self.assertRegex(request_id, r"^req-[0-9a-f]{32}$")
        self.assertEqual(
            payload,
            {
                "code": code,
                "message": message,
                "retryable": False,
                "requestId": request_id,
            },
        )
        return payload


class BatchJobApiTestCase(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.processor = _ControlledProcessor()
        self.logger = _CapturingLogger()
        self.jobs = RecordingJobService(
            Path(self.temporary.name),
            processor=self.processor,
            supported_languages=("en",),
            now=lambda: "2026-07-14T21:10:00Z",
        )
        self.server = create_server(
            ServerSettings(host="127.0.0.1", port=0),
            logger=self.logger,
            job_service=self.jobs,
        )
        self.assertIsInstance(self.server, ThreadingHTTPServer)
        host, port = self.server.server_address[:2]
        self.base_url = f"http://{host}:{port}"
        self.thread = threading.Thread(
            target=self.server.serve_forever,
            kwargs={"poll_interval": 0.01},
            daemon=True,
        )
        self.thread.start()

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        self.assertFalse(self.thread.is_alive())
        self.temporary.cleanup()

    def _request(
        self,
        path: str,
        *,
        method: str = "GET",
        headers: dict[str, str] | None = None,
        data: bytes | None = None,
    ) -> tuple[int, Any, dict[str, object]]:
        request = Request(
            f"{self.base_url}{path}",
            data=data,
            headers=headers or {},
            method=method,
        )
        try:
            response = urlopen(request, timeout=2)
        except HTTPError as error:
            response = error
        with response:
            body = response.read()
            return response.status, response.headers, json.loads(body)
