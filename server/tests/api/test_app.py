import io
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import tempfile
import threading
import time
import unittest
from concurrent.futures import Future, ThreadPoolExecutor
from contextlib import redirect_stderr
from http.server import HTTPServer, ThreadingHTTPServer
from typing import Any
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from yap_server.api.app import MAX_CONCURRENT_REQUEST_THREADS, create_server
from yap_server.config import ServerSettings
from yap_server.jobs import RecordingJobService
from yap_server.pools.batch_asr import BatchAsrJob


MAX_REQUEST_BODY_BYTES = 1024 * 1024


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


class ServerSettingsTests(unittest.TestCase):
    def test_environment_defaults_to_the_loopback_service_address(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            self.assertEqual(
                ServerSettings.from_env(),
                ServerSettings(host="127.0.0.1", port=18765),
            )

    def test_environment_reads_an_explicit_loopback_host_and_port(self) -> None:
        with patch.dict(
            os.environ,
            {"YAP_SERVER_HOST": "::1", "YAP_SERVER_PORT": "28765"},
            clear=True,
        ):
            self.assertEqual(
                ServerSettings.from_env(),
                ServerSettings(host="::1", port=28765),
            )

    def test_private_bind_requires_the_exact_opt_in(self) -> None:
        for allow_value in (None, "0", "true"):
            environment = {"YAP_SERVER_HOST": "192.168.50.1"}
            if allow_value is not None:
                environment["YAP_SERVER_ALLOW_PRIVATE_BIND"] = allow_value
            with self.subTest(allow_value=allow_value):
                with patch.dict(os.environ, environment, clear=True):
                    with self.assertRaisesRegex(
                        ValueError, "YAP_SERVER_ALLOW_PRIVATE_BIND=1"
                    ):
                        ServerSettings.from_env()

    def test_private_bind_is_allowed_after_explicit_opt_in(self) -> None:
        with patch.dict(
            os.environ,
            {
                "YAP_SERVER_HOST": "192.168.50.1",
                "YAP_SERVER_PORT": "18766",
                "YAP_SERVER_ALLOW_PRIVATE_BIND": "1",
            },
            clear=True,
        ):
            self.assertEqual(
                ServerSettings.from_env(),
                ServerSettings(host="192.168.50.1", port=18766),
            )

    def test_wildcard_bind_is_rejected_without_opt_in(self) -> None:
        with patch.dict(
            os.environ,
            {"YAP_SERVER_HOST": "0.0.0.0"},
            clear=True,
        ):
            with self.assertRaisesRegex(
                ValueError, "YAP_SERVER_ALLOW_PRIVATE_BIND=1"
            ):
                ServerSettings.from_env()

    def test_invalid_environment_port_is_rejected(self) -> None:
        for port in ("not-a-port", "-1", "65536"):
            with self.subTest(port=port):
                with patch.dict(
                    os.environ,
                    {"YAP_SERVER_PORT": port},
                    clear=True,
                ):
                    with self.assertRaisesRegex(ValueError, "YAP_SERVER_PORT"):
                        ServerSettings.from_env()


class HealthServiceTests(unittest.TestCase):
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

    def test_health_returns_contract_json_and_no_store_headers(self) -> None:
        status, headers, body = self._request("/v1/health")

        self.assertEqual(status, 200)
        self.assert_json_headers(headers, body)
        self.assertEqual(
            json.loads(body),
            {
                "service": "yap-server",
                "status": "ok",
                "apiVersion": "1",
                "auth": "not_configured",
                "capabilities": {
                    "batchJobs": False,
                    "liveStreaming": False,
                    "jobStatus": False,
                },
            },
        )

    def test_unknown_route_returns_the_stable_json_error(self) -> None:
        status, headers, body = self._request("/v1/unknown")

        self.assert_error(
            status,
            headers,
            body,
            expected_status=404,
            code="NOT_FOUND",
            message="Route not found.",
        )

    def test_non_get_health_method_returns_405(self) -> None:
        for method in ("POST", "TRACE"):
            with self.subTest(method=method):
                status, headers, body = self._request(
                    "/v1/health",
                    method=method,
                    data=b"" if method == "POST" else None,
                )

                self.assertEqual(headers["Allow"], "GET")
                self.assert_error(
                    status,
                    headers,
                    body,
                    expected_status=405,
                    code="METHOD_NOT_ALLOWED",
                    message="Method not allowed for this route.",
                )

    def test_oversized_request_is_rejected_before_body_read(self) -> None:
        status, headers, body = self._request(
            "/v1/health",
            method="POST",
            headers={"Content-Length": str(MAX_REQUEST_BODY_BYTES + 1)},
        )

        self.assert_error(
            status,
            headers,
            body,
            expected_status=413,
            code="REQUEST_TOO_LARGE",
            message="Request body exceeds the 1048576-byte limit.",
        )

    def test_contract_only_routes_return_501(self) -> None:
        routes = (
            ("POST", "/v1/jobs"),
            ("GET", "/v1/jobs/job-01"),
            ("DELETE", "/v1/jobs/job-01"),
            ("PUT", "/v1/jobs/job-01/chunks/mic/0-15"),
            ("POST", "/v1/jobs/job-01/commit"),
            ("GET", "/v1/live"),
        )
        for method, path in routes:
            with self.subTest(method=method, path=path):
                status, headers, body = self._request(path, method=method)
                self.assert_error(
                    status,
                    headers,
                    body,
                    expected_status=501,
                    code="NOT_IMPLEMENTED",
                    message="This route is contract-only in Phase 3.",
                )

    def test_invalid_chunk_range_is_not_a_contract_route(self) -> None:
        for suffix in ("not-a-range", "0-15-99", "-1-15"):
            with self.subTest(suffix=suffix):
                status, headers, body = self._request(
                    f"/v1/jobs/job-01/chunks/mic/{suffix}",
                    method="PUT",
                )
                self.assert_error(
                    status,
                    headers,
                    body,
                    expected_status=404,
                    code="NOT_FOUND",
                    message="Route not found.",
                )

    def test_request_logging_is_one_bounded_structured_line(self) -> None:
        path = "/v1/" + ("x" * 5000)
        status, _, _ = self._request(path)

        self.assertEqual(status, 404)
        self.assertEqual(len(self.logger.messages), 1)
        line = self.logger.messages[0]
        self.assertNotIn("\n", line)
        self.assertLessEqual(len(line), 1024)
        event = json.loads(line)
        self.assertEqual(event["event"], "http_request")
        self.assertEqual(event["method"], "GET")
        self.assertEqual(event["status"], 404)
        self.assertLessEqual(len(event["path"]), 513)

    def test_request_logging_redacts_query_and_absolute_target_secrets(self) -> None:
        status, _, body = self._request(
            "/v1/health?token=relative-secret"
        )
        absolute_response = self._raw_request(
            b"GET http://user:absolute-secret@127.0.0.1/v1/health"
            b"?token=query-secret HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
        )
        absolute_status, absolute_body = self._parse_raw_json_response(
            absolute_response
        )

        expected_health = {
            "service": "yap-server",
            "status": "ok",
            "apiVersion": "1",
            "auth": "not_configured",
            "capabilities": {
                "batchJobs": False,
                "liveStreaming": False,
                "jobStatus": False,
            },
        }
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body), expected_health)
        self.assertEqual(absolute_status, 200)
        self.assertEqual(absolute_body, expected_health)
        self.assertEqual(len(self.logger.messages), 2)

        serialized_logs = "\n".join(self.logger.messages)
        for secret in (
            "relative-secret",
            "absolute-secret",
            "query-secret",
            "user",
            "?token=",
        ):
            self.assertNotIn(secret, serialized_logs)

        for line in self.logger.messages:
            self.assertNotIn("\n", line)
            self.assertLessEqual(len(line), 1024)
            event = json.loads(line)
            self.assertEqual(event["path"], "/v1/health")
            self.assertRegex(event["requestId"], r"^req-[0-9a-f]{32}$")

    def test_request_log_bound_includes_json_escape_expansion(self) -> None:
        request_target = b"/v1/" + (b"\x80" * 600)
        response = self._raw_request(
            b"GET "
            + request_target
            + b" HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
        )

        self.assertIn(b" 404 ", response.split(b"\r\n", 1)[0])
        self.assertEqual(len(self.logger.messages), 1)
        self.assertLessEqual(len(self.logger.messages[0]), 1024)
        json.loads(self.logger.messages[0])

    def test_malformed_request_target_returns_stable_json_error(self) -> None:
        response = self._raw_request(
            b"GET http://[bad/v1/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
        )

        head, body = response.split(b"\r\n\r\n", 1)
        self.assertIn(b" 400 ", head.split(b"\r\n", 1)[0])
        payload = json.loads(body)
        request_id = payload.get("requestId")
        self.assertRegex(request_id, r"^req-[0-9a-f]{32}$")
        self.assertEqual(
            payload,
            {
                "code": "INVALID_REQUEST_TARGET",
                "message": "Request target is invalid.",
                "retryable": False,
                "requestId": request_id,
            },
        )
        self.assertEqual(len(self.logger.messages), 1)

    def test_client_disconnect_is_logged_without_default_traceback(self) -> None:
        host, port = self.server.server_address[:2]
        request_logged = threading.Event()
        release_response = threading.Event()
        barrier_timed_out = threading.Event()
        real_info = self.logger.info

        def log_then_wait_for_disconnect(message: str) -> None:
            real_info(message)
            request_logged.set()
            if not release_response.wait(timeout=2):
                barrier_timed_out.set()

        stderr = io.StringIO()
        with redirect_stderr(stderr):
            client = socket.create_connection((host, port), timeout=2)
            try:
                with patch.object(
                    self.logger,
                    "info",
                    side_effect=log_then_wait_for_disconnect,
                ):
                    client.setsockopt(
                        socket.SOL_SOCKET,
                        socket.SO_LINGER,
                        struct.pack("HH" if os.name == "nt" else "ii", 1, 0),
                    )
                    client.sendall(
                        b"GET /v1/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
                    )
                    self.assertTrue(
                        request_logged.wait(timeout=2),
                        "server did not reach the pre-write logging barrier",
                    )
                    client.close()
            finally:
                client.close()
                release_response.set()

            self.assertFalse(
                barrier_timed_out.is_set(),
                "server timed out waiting for the client disconnect",
            )
            self.assertEqual(len(self.logger.messages), 1)
            disconnect_log = self.logger.messages[0]
            self.assertLessEqual(len(disconnect_log), 1024)
            self.assertEqual(json.loads(disconnect_log)["status"], 200)
            self.assertEqual(stderr.getvalue(), "")

            status, _, _ = self._request("/v1/health")
            self.assertEqual(status, 200)
            self.assertEqual(len(self.logger.messages), 2)

        self.assertEqual(stderr.getvalue(), "")

    def test_unexpected_request_failure_is_generic_without_default_traceback(self) -> None:
        private_path = "C:/private/recordings/patient-audio.wav"
        stderr = io.StringIO()
        with (
            redirect_stderr(stderr),
            patch(
                "yap_server.api.app.health",
                side_effect=RuntimeError(private_path),
            ),
        ):
            response = self._raw_request(
                b"GET /v1/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
            )

        self.assertEqual(response, b"")
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(
            self.logger.messages,
            ['{"event":"http_request_failure","status":500}'],
        )
        self.assertNotIn(private_path, "".join(self.logger.messages))

        status, _, _ = self._request("/v1/health")
        self.assertEqual(status, 200)

    def test_partial_headers_time_out_without_blocking_the_server(self) -> None:
        host, port = self.server.server_address[:2]
        stalled = socket.create_connection((host, port), timeout=2)
        stalled.settimeout(5)
        stalled.sendall(
            b"GET /v1/health HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            b"X-Stall: "
        )
        try:
            status, _, _ = self._request("/v1/health", timeout=5)
            stalled_response = bytearray()
            while chunk := stalled.recv(4096):
                stalled_response.extend(chunk)
        finally:
            stalled.close()

        self.assertEqual(status, 200)
        head, body = bytes(stalled_response).split(b"\r\n\r\n", 1)
        self.assertIn(b" 408 ", head.split(b"\r\n", 1)[0])
        payload = json.loads(body)
        self.assertEqual(payload["code"], "REQUEST_TIMEOUT")
        self.assertEqual(payload["retryable"], False)
        self.assertEqual(
            [json.loads(line)["status"] for line in self.logger.messages],
            [408, 200],
        )

    def test_partial_request_line_times_out_without_default_traceback(self) -> None:
        host, port = self.server.server_address[:2]
        stalled = socket.create_connection((host, port), timeout=2)
        stalled.settimeout(5)
        stalled.sendall(b"GET /v1/health")
        stderr = io.StringIO()
        try:
            with redirect_stderr(stderr):
                status, _, _ = self._request("/v1/health", timeout=5)
                stalled_response = bytearray()
                while chunk := stalled.recv(4096):
                    stalled_response.extend(chunk)
        finally:
            stalled.close()

        self.assertEqual(status, 200)
        self.assertEqual(stderr.getvalue(), "")
        head, body = bytes(stalled_response).split(b"\r\n\r\n", 1)
        self.assertIn(b" 408 ", head.split(b"\r\n", 1)[0])
        self.assertEqual(json.loads(body)["code"], "REQUEST_TIMEOUT")
        self.assertEqual(
            [json.loads(line)["status"] for line in self.logger.messages],
            [408, 200],
        )

    def test_simple_http_request_is_rejected_without_default_traceback(self) -> None:
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            response = self._raw_request(b"GET /v1/health\r\n")

        self.assertEqual(stderr.getvalue(), "")
        head, body = response.split(b"\r\n\r\n", 1)
        self.assertIn(b" 505 ", head.split(b"\r\n", 1)[0])
        payload = json.loads(body)
        request_id = payload.get("requestId")
        self.assertEqual(
            payload,
            {
                "code": "HTTP_VERSION_NOT_SUPPORTED",
                "message": "HTTP/1.0 or HTTP/1.1 is required.",
                "retryable": False,
                "requestId": request_id,
            },
        )
        self.assertEqual(len(self.logger.messages), 1)
        self.assertEqual(json.loads(self.logger.messages[0])["status"], 505)

    def test_slow_drip_has_a_wall_clock_deadline_and_bounded_stop(self) -> None:
        host, port = self.server.server_address[:2]
        stalled = socket.create_connection((host, port), timeout=2)
        stop_drip = threading.Event()
        request = (
            b"GET /v1/health HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            b"X-Slow: never-completes\r\n\r\n"
        )
        stalled.sendall(request[:1])

        def drip() -> None:
            for byte in request[1:]:
                if stop_drip.wait(0.25):
                    return
                try:
                    stalled.sendall(bytes((byte,)))
                except OSError:
                    return

        drip_thread = threading.Thread(target=drip, daemon=True)
        drip_thread.start()
        time.sleep(0.05)
        started = time.monotonic()
        try:
            status, _, _ = self._request("/v1/health", timeout=4)
            elapsed = time.monotonic() - started
        finally:
            stop_drip.set()
            stalled.close()
            drip_thread.join(timeout=1)

        self.assertEqual(status, 200)
        self.assertLess(elapsed, 3.5)
        self.assertEqual(
            [json.loads(line)["status"] for line in self.logger.messages],
            [408, 200],
        )

        shutdown_started = time.monotonic()
        self.server.shutdown()
        self.thread.join(timeout=1)
        self.assertLess(time.monotonic() - shutdown_started, 1)
        self.assertFalse(self.thread.is_alive())

    def test_only_http_1_0_and_1_1_are_supported(self) -> None:
        versions = ("HTTP/0.8", "HTTP/0.9", "HTTP/1.2", "HTTP/1.9")
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            for version in versions:
                with self.subTest(version=version):
                    response = self._raw_request(
                        f"GET /v1/health {version}\r\n"
                        "Host: 127.0.0.1\r\n"
                        "Connection: close\r\n\r\n".encode("ascii")
                    )
                    status, payload = self._parse_raw_json_response(response)
                    self.assertEqual(status, 505)
                    self.assertEqual(payload["code"], "HTTP_VERSION_NOT_SUPPORTED")
                    self.assertEqual(payload["retryable"], False)

        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(
            [json.loads(line)["status"] for line in self.logger.messages],
            [505, 505, 505, 505],
        )

    def test_pre_dispatch_parse_errors_are_stable_and_none_safe(self) -> None:
        cases = (
            (b"BROKEN\r\n\r\n", 400, "HTTP_ERROR"),
            (
                b"GET /v1/health HTTP/1.1 EXTRA\r\n"
                b"Host: 127.0.0.1\r\n\r\n",
                400,
                "HTTP_ERROR",
            ),
            (
                b"GET /v1/health HTTP/2.0\r\nHost: 127.0.0.1\r\n\r\n",
                505,
                "HTTP_VERSION_NOT_SUPPORTED",
            ),
        )
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            for request, expected_status, expected_code in cases:
                with self.subTest(request=request):
                    response = self._raw_request(request)
                    status, payload = self._parse_raw_json_response(response)
                    self.assertEqual(status, expected_status)
                    self.assertEqual(payload["code"], expected_code)
                    self.assertEqual(payload["retryable"], False)
                    self.assertRegex(payload["requestId"], r"^req-[0-9a-f]{32}$")

        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(
            [json.loads(line)["status"] for line in self.logger.messages],
            [400, 400, 505],
        )


class BatchJobApiTests(unittest.TestCase):
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

    def test_batch_create_requires_one_idempotency_key(self) -> None:
        status, _, payload = self._request(
            "/v1/jobs",
            method="POST",
            headers={"Content-Type": "application/json"},
            data=json.dumps(_phase5_job_request()).encode("utf-8"),
        )

        self.assertEqual(status, 400)
        self.assertEqual(payload["code"], "IDEMPOTENCY_KEY_REQUIRED")

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


class BoundedBatchJobServerTests(unittest.TestCase):
    def test_storage_maintenance_failure_stays_private_and_keeps_serving(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            service = RecordingJobService(
                Path(temporary),
                processor=_ControlledProcessor(),
                supported_languages=("en",),
                now=lambda: "2026-07-14T21:00:00Z",
            )
            server = create_server(
                ServerSettings(host="127.0.0.1", port=0),
                logger=_CapturingLogger(),
                job_service=service,
            )
            serving = threading.Thread(
                target=server.serve_forever,
                kwargs={"poll_interval": 0.01},
                daemon=True,
            )
            attempted = threading.Event()
            private_path = "C:/private/recordings/patient-audio.wav"

            def fail_maintenance() -> None:
                attempted.set()
                raise OSError(f"could not prune {private_path}")

            stderr = io.StringIO()
            try:
                with redirect_stderr(stderr):
                    with patch.object(
                        service,
                        "prune_expired",
                        side_effect=fail_maintenance,
                    ):
                        serving.start()
                        self.assertTrue(attempted.wait(timeout=1))
                        self.assertTrue(serving.is_alive())
            finally:
                server.shutdown()
                serving.join(timeout=2)
                server.server_close()

            self.assertFalse(serving.is_alive())
            self.assertNotIn(private_path, stderr.getvalue())

    def test_idle_server_runs_terminal_retention_without_a_new_request(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            clock = {"now": "2026-07-14T21:00:00Z"}
            service = RecordingJobService(
                root,
                processor=_ControlledProcessor(),
                supported_languages=("en",),
                now=lambda: clock["now"],
            )
            request = _phase5_job_request()
            request["metadata"]["retentionExpiresAtUtc"] = "2026-07-15T00:00:00Z"
            created = service.create(request)
            service.cancel(created["jobId"])
            expired_root = root / "jobs" / created["jobId"]
            server = create_server(
                ServerSettings(host="127.0.0.1", port=0),
                logger=_CapturingLogger(),
                job_service=service,
            )
            serving = threading.Thread(
                target=server.serve_forever,
                kwargs={"poll_interval": 0.01},
                daemon=True,
            )
            clock["now"] = "2026-07-16T00:00:00Z"
            serving.start()
            try:
                deadline = time.monotonic() + 1
                while expired_root.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertFalse(expired_root.exists())
            finally:
                server.shutdown()
                serving.join(timeout=2)
                server.server_close()

    def test_batch_request_threads_are_bounded_under_concurrent_load(self) -> None:
        maximum_threads = MAX_CONCURRENT_REQUEST_THREADS
        service = _BlockingStatusService(maximum_threads)
        server = create_server(
            ServerSettings(host="127.0.0.1", port=0),
            logger=_CapturingLogger(),
            job_service=service,  # type: ignore[arg-type]
        )
        host, port = server.server_address[:2]
        serving = threading.Thread(
            target=server.serve_forever,
            kwargs={"poll_interval": 0.01},
            daemon=True,
        )
        serving.start()

        def request_status(index: int) -> int:
            with urlopen(
                f"http://{host}:{port}/v1/jobs/job-load-{index}",
                timeout=5,
            ) as response:
                response.read()
                return response.status

        try:
            with ThreadPoolExecutor(max_workers=maximum_threads + 2) as requests:
                responses = [
                    requests.submit(request_status, index)
                    for index in range(maximum_threads + 2)
                ]
                self.assertTrue(service.saturated.wait(timeout=2))
                time.sleep(0.1)
                self.assertLessEqual(service.maximum_active, maximum_threads)
                service.release.set()
                self.assertEqual(
                    [response.result(timeout=5) for response in responses],
                    [200] * (maximum_threads + 2),
                )
        finally:
            service.release.set()
            server.shutdown()
            server.server_close()
            serving.join(timeout=2)
            self.assertFalse(serving.is_alive())


if __name__ == "__main__":
    unittest.main()
