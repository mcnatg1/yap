from __future__ import annotations

import io
import json
import logging
import re
import socket
import sys
import time
from functools import partial
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, HTTPServer
from ipaddress import ip_address
from typing import Any, Mapping
from urllib.parse import urlsplit
from uuid import uuid4

from yap_server.api.health import health
from yap_server.config import ServerSettings
from yap_server.config.settings import ensure_bind_is_allowed


MAX_REQUEST_BODY_BYTES = 1024 * 1024
MAX_LOG_METHOD_CHARS = 16
MAX_LOG_PATH_CHARS = 128
REQUEST_IO_TIMEOUT_SECONDS = 2.0
REQUEST_WALL_CLOCK_DEADLINE_SECONDS = 2.0
_SUPPORTED_HTTP_VERSIONS = frozenset({"HTTP/1.0", "HTTP/1.1"})

_REQUEST_LOGGER = logging.getLogger("yap_server.requests")
_JOB_PATH = re.compile(r"^/v1/jobs/[^/]+$")
_CHUNK_PATH = re.compile(r"^/v1/jobs/[^/]+/chunks/[^/]+/[0-9]+-[0-9]+$")
_COMMIT_PATH = re.compile(r"^/v1/jobs/[^/]+/commit$")


def _allowed_methods(path: str) -> frozenset[str] | None:
    if path == "/v1/health":
        return frozenset({"GET"})
    if path == "/v1/jobs":
        return frozenset({"POST"})
    if _JOB_PATH.fullmatch(path):
        return frozenset({"DELETE", "GET"})
    if _CHUNK_PATH.fullmatch(path):
        return frozenset({"PUT"})
    if _COMMIT_PATH.fullmatch(path):
        return frozenset({"POST"})
    if path == "/v1/live":
        return frozenset({"GET"})
    return None


def _bounded(value: str | None, maximum: int) -> str:
    if not isinstance(value, str):
        return ""
    if len(value) <= maximum:
        return value
    return value[:maximum] + "…"


def _sanitized_log_path(value: object) -> str:
    if not isinstance(value, str):
        return ""
    try:
        return urlsplit(value).path
    except ValueError:
        return ""


class _DeadlineSocketReader(io.RawIOBase):
    def __init__(self, connection: socket.socket, deadline: float) -> None:
        super().__init__()
        self._connection = connection
        self._deadline = deadline

    def readable(self) -> bool:
        return True

    def readinto(self, buffer: Any) -> int:
        remaining = self._deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("request wall-clock deadline exceeded")
        self._connection.settimeout(min(REQUEST_IO_TIMEOUT_SECONDS, remaining))
        return self._connection.recv_into(buffer)


class _HealthRequestHandler(BaseHTTPRequestHandler):
    server_version = "yap-server"
    sys_version = ""
    timeout = REQUEST_IO_TIMEOUT_SECONDS

    def __init__(
        self,
        *args: Any,
        request_logger: logging.Logger,
        **kwargs: Any,
    ) -> None:
        self._request_logger = request_logger
        self._request_id = f"req-{uuid4().hex}"
        self._request_logged = False
        self.requestline = ""
        self.request_version = ""
        self.command = ""
        super().__init__(*args, **kwargs)

    def setup(self) -> None:
        deadline = time.monotonic() + REQUEST_WALL_CLOCK_DEADLINE_SECONDS
        super().setup()
        original_rfile = self.rfile
        self.rfile = io.BufferedReader(
            _DeadlineSocketReader(self.connection, deadline)
        )
        original_rfile.close()

    def do_GET(self) -> None:
        self._dispatch()

    def do_POST(self) -> None:
        self._dispatch()

    def do_PUT(self) -> None:
        self._dispatch()

    def do_DELETE(self) -> None:
        self._dispatch()

    def do_CONNECT(self) -> None:
        self._dispatch()

    def do_PATCH(self) -> None:
        self._dispatch()

    def do_HEAD(self) -> None:
        self._dispatch()

    def do_OPTIONS(self) -> None:
        self._dispatch()

    def do_TRACE(self) -> None:
        self._dispatch()

    def _dispatch(self) -> None:
        if self.request_version not in _SUPPORTED_HTTP_VERSIONS:
            self._send_version_not_supported()
            return

        if not self._request_size_is_allowed():
            return

        try:
            path = urlsplit(self.path).path
        except ValueError:
            self._send_error(
                HTTPStatus.BAD_REQUEST,
                code="INVALID_REQUEST_TARGET",
                message="Request target is invalid.",
            )
            return
        allowed_methods = _allowed_methods(path)
        if allowed_methods is None:
            self._send_error(
                HTTPStatus.NOT_FOUND,
                code="NOT_FOUND",
                message="Route not found.",
            )
            return

        if self.command not in allowed_methods:
            self._send_error(
                HTTPStatus.METHOD_NOT_ALLOWED,
                code="METHOD_NOT_ALLOWED",
                message="Method not allowed for this route.",
                headers={"Allow": ", ".join(sorted(allowed_methods))},
            )
            return

        if path == "/v1/health":
            self._send_json(HTTPStatus.OK, health())
            return

        self._send_error(
            HTTPStatus.NOT_IMPLEMENTED,
            code="NOT_IMPLEMENTED",
            message="This route is contract-only in Phase 3.",
        )

    def _request_size_is_allowed(self) -> bool:
        if self.headers.get("Transfer-Encoding") is not None:
            self.close_connection = True
            self._send_error(
                HTTPStatus.BAD_REQUEST,
                code="INVALID_REQUEST_BODY",
                message="Transfer-encoded request bodies are not supported.",
            )
            return False

        content_lengths = self.headers.get_all("Content-Length", [])
        if not content_lengths:
            return True
        if len(content_lengths) != 1:
            self.close_connection = True
            self._send_error(
                HTTPStatus.BAD_REQUEST,
                code="INVALID_CONTENT_LENGTH",
                message="Content-Length must appear exactly once.",
            )
            return False

        try:
            content_length = int(content_lengths[0], 10)
        except ValueError:
            content_length = -1
        if content_length < 0:
            self.close_connection = True
            self._send_error(
                HTTPStatus.BAD_REQUEST,
                code="INVALID_CONTENT_LENGTH",
                message="Content-Length must be a non-negative integer.",
            )
            return False
        if content_length > MAX_REQUEST_BODY_BYTES:
            self.close_connection = True
            self._send_error(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                code="REQUEST_TOO_LARGE",
                message="Request body exceeds the 1048576-byte limit.",
            )
            return False
        return True

    def _send_error(
        self,
        status: HTTPStatus,
        *,
        code: str,
        message: str,
        headers: Mapping[str, str] | None = None,
    ) -> None:
        self._send_json(
            status,
            {
                "code": code,
                "message": message,
                "retryable": False,
                "requestId": self._request_id,
            },
            headers=headers,
        )

    def _send_version_not_supported(self) -> None:
        self.request_version = "HTTP/1.0"
        self.close_connection = True
        self._send_error(
            HTTPStatus.HTTP_VERSION_NOT_SUPPORTED,
            code="HTTP_VERSION_NOT_SUPPORTED",
            message="HTTP/1.0 or HTTP/1.1 is required.",
        )

    def _send_json(
        self,
        status: HTTPStatus,
        payload: Mapping[str, object],
        *,
        headers: Mapping[str, str] | None = None,
    ) -> None:
        body = json.dumps(
            payload,
            ensure_ascii=True,
            separators=(",", ":"),
        ).encode("utf-8")
        self._log_structured_request(int(status))
        try:
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Content-Length", str(len(body)))
            if headers:
                for name, value in headers.items():
                    self.send_header(name, value)
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)
        except OSError:
            self.close_connection = True

    def _log_structured_request(self, status: int) -> None:
        if self._request_logged:
            return
        self._request_logged = True
        event = {
            "event": "http_request",
            "method": _bounded(getattr(self, "command", ""), MAX_LOG_METHOD_CHARS),
            "path": _bounded(
                _sanitized_log_path(getattr(self, "path", "")),
                MAX_LOG_PATH_CHARS,
            ),
            "status": status,
            "requestId": self._request_id,
        }
        self._request_logger.info(
            json.dumps(event, ensure_ascii=True, separators=(",", ":"))
        )

    def send_error(
        self,
        code: int,
        message: str | None = None,
        explain: str | None = None,
    ) -> None:
        del message, explain
        try:
            status = HTTPStatus(code)
        except ValueError:
            status = HTTPStatus.INTERNAL_SERVER_ERROR
        if status == HTTPStatus.HTTP_VERSION_NOT_SUPPORTED:
            self._send_version_not_supported()
            return
        if self.request_version not in _SUPPORTED_HTTP_VERSIONS:
            self.request_version = "HTTP/1.0"
            self.close_connection = True
        self._send_error(
            status,
            code="HTTP_ERROR",
            message="The HTTP request could not be processed.",
        )

    def log_message(self, format: str, *args: Any) -> None:
        del format, args

    def log_error(self, format: str, *args: Any) -> None:
        del args
        if format == "Request timed out: %r" and not self._request_logged:
            self._send_error(
                HTTPStatus.REQUEST_TIMEOUT,
                code="REQUEST_TIMEOUT",
                message="Request headers were not completed in time.",
            )


class _YapHTTPServer(HTTPServer):
    def handle_error(self, request: Any, client_address: Any) -> None:
        if isinstance(sys.exc_info()[1], ConnectionError):
            return
        super().handle_error(request, client_address)


class _IPv6HTTPServer(_YapHTTPServer):
    address_family = socket.AF_INET6


def _server_type(host: str) -> type[HTTPServer]:
    try:
        if ip_address(host).version == 6:
            return _IPv6HTTPServer
    except ValueError:
        pass
    return _YapHTTPServer


def create_server(
    settings: ServerSettings,
    *,
    logger: logging.Logger | None = None,
) -> HTTPServer:
    ensure_bind_is_allowed(settings.host)
    handler = partial(
        _HealthRequestHandler,
        request_logger=logger or _REQUEST_LOGGER,
    )
    return _server_type(settings.host)((settings.host, settings.port), handler)


def serve(settings: ServerSettings) -> None:
    with create_server(settings) as server:
        server.serve_forever()
