import io
import json
import os
import socket
import struct
import threading
import time
import unittest
from contextlib import redirect_stderr
from http.server import HTTPServer, ThreadingHTTPServer
from typing import Any
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from yap_server.api.app import create_server
from yap_server.config import ServerSettings


MAX_REQUEST_BODY_BYTES = 1024 * 1024


class _CapturingLogger:
    def __init__(self) -> None:
        self.messages: list[str] = []

    def info(self, message: str) -> None:
        self.messages.append(message)


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
        attempts = 5
        stderr = io.StringIO()
        with redirect_stderr(stderr):
            for _ in range(attempts):
                with socket.create_connection((host, port), timeout=2) as client:
                    client.setsockopt(
                        socket.SOL_SOCKET,
                        socket.SO_LINGER,
                        struct.pack("HH" if os.name == "nt" else "ii", 1, 0),
                    )
                    client.sendall(
                        b"GET /v1/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n"
                    )

            deadline = time.monotonic() + 2
            while (
                len(self.logger.messages) < attempts
                and time.monotonic() < deadline
            ):
                time.sleep(0.01)

        self.assertEqual(len(self.logger.messages), attempts)
        self.assertEqual(stderr.getvalue(), "")
        for line in self.logger.messages:
            self.assertLessEqual(len(line), 1024)
            self.assertEqual(json.loads(line)["status"], 200)

        status, _, _ = self._request("/v1/health")
        self.assertEqual(status, 200)
        self.assertEqual(len(self.logger.messages), attempts + 1)

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


if __name__ == "__main__":
    unittest.main()
