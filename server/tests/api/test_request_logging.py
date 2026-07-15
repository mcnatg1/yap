import io
import json
import os
import socket
import struct
import threading
import time
from contextlib import redirect_stderr
from unittest.mock import patch

from .api_fixtures import HealthServerTestCase


class RequestLoggingTests(HealthServerTestCase):
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
