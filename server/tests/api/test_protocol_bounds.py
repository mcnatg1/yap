import io
import json
import socket
import threading
import time
from contextlib import redirect_stderr

from .api_fixtures import HealthServerTestCase


class ProtocolBoundsTests(HealthServerTestCase):
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
