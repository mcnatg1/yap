from __future__ import annotations

import io
import json
import logging
import re
import socket
import sys
import threading
import time
from functools import partial
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, HTTPServer, ThreadingHTTPServer
from ipaddress import ip_address
from typing import Any, Mapping
from urllib.parse import urlsplit
from uuid import uuid4

from yap_server.api.health import health
from yap_server.config import ServerSettings
from yap_server.config.settings import ensure_bind_is_allowed
from yap_server.jobs import JobServiceError, RecordingJobService


MAX_REQUEST_BODY_BYTES = 1024 * 1024
_BODY_DRAIN_READ_BYTES = 64 * 1024
MAX_LOG_METHOD_CHARS = 16
MAX_LOG_PATH_CHARS = 128
REQUEST_IO_TIMEOUT_SECONDS = 2.0
REQUEST_WALL_CLOCK_DEADLINE_SECONDS = 2.0
MAX_CONCURRENT_REQUEST_THREADS = 8
REQUEST_THREAD_ACQUIRE_TIMEOUT_SECONDS = 2.0
MAINTENANCE_INTERVAL_SECONDS = 60.0
_SUPPORTED_HTTP_VERSIONS = frozenset({"HTTP/1.0", "HTTP/1.1"})

_REQUEST_LOGGER = logging.getLogger("yap_server.requests")
_PATH_ID = r"[A-Za-z0-9_-]+"
_JOB_PATH = re.compile(rf"^/v1/jobs/(?P<job_id>{_PATH_ID})$")
_RESULT_PATH = re.compile(rf"^/v1/jobs/(?P<job_id>{_PATH_ID})/result$")
_CHUNK_PATH = re.compile(
    rf"^/v1/jobs/(?P<job_id>{_PATH_ID})/chunks/"
    rf"(?P<track_id>{_PATH_ID})/(?P<sequence_start>[0-9]+)-"
    rf"(?P<sequence_end>[0-9]+)$"
)
_COMMIT_PATH = re.compile(rf"^/v1/jobs/(?P<job_id>{_PATH_ID})/commit$")


def _allowed_methods(path: str) -> frozenset[str] | None:
    if path == "/v1/health":
        return frozenset({"GET"})
    if path == "/v1/jobs":
        return frozenset({"POST"})
    if _JOB_PATH.fullmatch(path):
        return frozenset({"DELETE", "GET"})
    if _RESULT_PATH.fullmatch(path):
        return frozenset({"GET"})
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
        job_service: RecordingJobService | None,
        **kwargs: Any,
    ) -> None:
        self._request_logger = request_logger
        self._job_service = job_service
        self._request_id = f"req-{uuid4().hex}"
        self._request_logged = False
        self._content_length: int | None = None
        self._body_bytes_read = 0
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
            self._send_json(
                HTTPStatus.OK,
                health(batch_jobs=self._job_service is not None),
            )
            return

        if self._job_service is not None and path != "/v1/live":
            self._dispatch_job_request(path)
            return

        self._send_error(
            HTTPStatus.NOT_IMPLEMENTED,
            code="NOT_IMPLEMENTED",
            message="This route is contract-only in Phase 3.",
        )

    def _dispatch_job_request(self, path: str) -> None:
        assert self._job_service is not None
        try:
            if path == "/v1/jobs" and self.command == "POST":
                idempotency_key = self._required_header(
                    "Idempotency-Key",
                    code="IDEMPOTENCY_KEY_REQUIRED",
                    message="Job creation requires exactly one idempotency key.",
                )
                payload = self._read_json_body()
                self._send_json(
                    HTTPStatus.ACCEPTED,
                    self._job_service.create(
                        payload,
                        idempotency_key=idempotency_key,
                    ),
                )
                return

            chunk_match = _CHUNK_PATH.fullmatch(path)
            if chunk_match is not None and self.command == "PUT":
                if self.headers.get_content_type() != "application/octet-stream":
                    raise JobServiceError(
                        415,
                        "UNSUPPORTED_MEDIA_TYPE",
                        "Chunk uploads require application/octet-stream.",
                    )
                content_length = self._required_content_length()
                plan = self._job_service.prepare_chunk_upload(
                    chunk_match.group("job_id"),
                    track_id=chunk_match.group("track_id"),
                    sequence_start=int(chunk_match.group("sequence_start"), 10),
                    sequence_end=int(chunk_match.group("sequence_end"), 10),
                    idempotency_key=self._required_header("Idempotency-Key"),
                    content_sha256=self._required_header("X-Yap-Content-SHA256"),
                    audio_codec=self._required_header("X-Yap-Audio-Codec"),
                    sample_rate_hz=self._integer_header("X-Yap-Sample-Rate-Hz"),
                    channels=self._integer_header("X-Yap-Channels"),
                    content_length=content_length,
                )
                receipt = self._job_service.accept_chunk(
                    plan,
                    self._read_exact_body(content_length),
                )
                status = (
                    HTTPStatus.OK
                    if receipt.get("disposition") == "replayed"
                    else HTTPStatus.CREATED
                )
                self._send_json(status, receipt)
                return

            commit_match = _COMMIT_PATH.fullmatch(path)
            if commit_match is not None and self.command == "POST":
                payload = self._read_json_body()
                self._send_json(
                    HTTPStatus.ACCEPTED,
                    self._job_service.commit(commit_match.group("job_id"), payload),
                )
                return

            result_match = _RESULT_PATH.fullmatch(path)
            if result_match is not None and self.command == "GET":
                self._send_json(
                    HTTPStatus.OK,
                    self._job_service.get_result(result_match.group("job_id")),
                )
                return

            job_match = _JOB_PATH.fullmatch(path)
            if job_match is not None and self.command == "DELETE":
                self._send_json(
                    HTTPStatus.ACCEPTED,
                    self._job_service.cancel(job_match.group("job_id")),
                )
                return
            if job_match is not None and self.command == "GET":
                self._send_json(
                    HTTPStatus.OK,
                    self._job_service.get(job_match.group("job_id")),
                )
                return
        except JobServiceError as error:
            self._send_error(
                HTTPStatus(error.status),
                code=error.code,
                message=error.message,
                retryable=error.retryable,
            )
            return
        except KeyError:
            self._send_error(
                HTTPStatus.NOT_FOUND,
                code="JOB_NOT_FOUND",
                message="Recording job not found.",
            )
            return
        except TimeoutError:
            self.close_connection = True
            self._send_error(
                HTTPStatus.REQUEST_TIMEOUT,
                code="REQUEST_TIMEOUT",
                message="The bounded request did not complete in time.",
                retryable=True,
            )
            return
        except ConnectionError:
            self.close_connection = True
            return
        except OSError:
            self._send_error(
                HTTPStatus.INTERNAL_SERVER_ERROR,
                code="SERVER_STORAGE_ERROR",
                message="Private recording storage could not complete the request.",
                retryable=True,
            )
            return
        except (TypeError, ValueError):
            self._send_error(
                HTTPStatus.BAD_REQUEST,
                code="INVALID_REQUEST_BODY",
                message="Request body does not match the operation contract.",
            )
            return

        self._send_error(
            HTTPStatus.NOT_IMPLEMENTED,
            code="NOT_IMPLEMENTED",
            message="This operation is not implemented in the Phase 5 batch slice.",
        )

    def _required_content_length(self) -> int:
        if self._content_length is None:
            raise JobServiceError(
                400,
                "CONTENT_LENGTH_REQUIRED",
                "A bounded Content-Length is required.",
            )
        return self._content_length

    def _read_exact_body(self, content_length: int) -> bytes:
        body = self.rfile.read(content_length)
        self._body_bytes_read += len(body)
        if len(body) != content_length:
            self.close_connection = True
            raise JobServiceError(
                400,
                "INCOMPLETE_REQUEST_BODY",
                "Request body ended before Content-Length bytes were received.",
            )
        return body

    def _discard_unread_request_body(self) -> None:
        if self._content_length is None:
            return
        remaining = max(0, self._content_length - self._body_bytes_read)
        try:
            while remaining:
                body = self.rfile.read(min(remaining, _BODY_DRAIN_READ_BYTES))
                if not body:
                    self.close_connection = True
                    return
                consumed = len(body)
                self._body_bytes_read += consumed
                remaining -= consumed
        except (OSError, TimeoutError, ValueError):
            self.close_connection = True

    def _read_json_body(self) -> Mapping[str, object]:
        if self.headers.get_content_type() != "application/json":
            raise JobServiceError(
                415,
                "UNSUPPORTED_MEDIA_TYPE",
                "JSON operations require application/json.",
            )
        content_length = self._required_content_length()
        try:
            payload = json.loads(self._read_exact_body(content_length))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise JobServiceError(
                400,
                "INVALID_JSON",
                "Request body is not valid JSON.",
            ) from error
        if not isinstance(payload, dict):
            raise JobServiceError(
                400,
                "INVALID_REQUEST_BODY",
                "Request body must be a JSON object.",
            )
        return payload

    def _required_header(
        self,
        name: str,
        *,
        code: str = "INVALID_CHUNK_HEADERS",
        message: str = "Every required chunk identity header must appear exactly once.",
    ) -> str:
        values = self.headers.get_all(name, [])
        if len(values) != 1 or not values[0]:
            raise JobServiceError(
                400,
                code,
                message,
            )
        return values[0]

    def _integer_header(self, name: str) -> int:
        value = self._required_header(name)
        try:
            return int(value, 10)
        except ValueError as error:
            raise JobServiceError(
                400,
                "INVALID_CHUNK_HEADERS",
                "Chunk numeric headers must be decimal integers.",
            ) from error

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
            self._content_length = None
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
        self._content_length = content_length
        return True

    def _send_error(
        self,
        status: HTTPStatus,
        *,
        code: str,
        message: str,
        retryable: bool = False,
        headers: Mapping[str, str] | None = None,
    ) -> None:
        self._send_json(
            status,
            {
                "code": code,
                "message": message,
                "retryable": retryable,
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
        # On Windows, closing a socket with unread request bytes can emit a TCP
        # reset that discards an otherwise valid error response. Every declared
        # body is bounded before dispatch, so consume the bounded remainder at
        # the response boundary without allowing drain failures to mask it.
        self._discard_unread_request_body()
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
        del request, client_address
        if isinstance(sys.exc_info()[1], ConnectionError):
            return
        _log_unhandled_request_failure(self)


class _ThreadingYapHTTPServer(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = MAX_CONCURRENT_REQUEST_THREADS * 2

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        self._request_slots = threading.BoundedSemaphore(
            MAX_CONCURRENT_REQUEST_THREADS
        )
        self._job_service_for_maintenance: object | None = None
        self._next_maintenance_at = 0.0
        super().__init__(*args, **kwargs)

    def service_actions(self) -> None:
        super().service_actions()
        now = time.monotonic()
        if now < self._next_maintenance_at:
            return
        self._next_maintenance_at = now + MAINTENANCE_INTERVAL_SECONDS
        maintenance = getattr(self._job_service_for_maintenance, "prune_expired", None)
        if callable(maintenance):
            try:
                maintenance()
            except OSError:
                # Retention remains pending and is retried on the next bounded
                # maintenance interval. Do not let filesystem details escape
                # through the server loop's default traceback handling.
                pass

    def process_request(self, request: Any, client_address: Any) -> None:
        if not self._request_slots.acquire(
            timeout=REQUEST_THREAD_ACQUIRE_TIMEOUT_SECONDS
        ):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self._request_slots.release()
            raise

    def process_request_thread(self, request: Any, client_address: Any) -> None:
        try:
            super().process_request_thread(request, client_address)
        finally:
            self._request_slots.release()

    def handle_error(self, request: Any, client_address: Any) -> None:
        del request, client_address
        if isinstance(sys.exc_info()[1], ConnectionError):
            return
        _log_unhandled_request_failure(self)


def _log_unhandled_request_failure(server: HTTPServer) -> None:
    logger = getattr(server, "_request_error_logger", _REQUEST_LOGGER)
    try:
        logger.info(
            json.dumps(
                {"event": "http_request_failure", "status": 500},
                ensure_ascii=True,
                separators=(",", ":"),
            )
        )
    except Exception:
        # Error reporting is itself an outer trust boundary. A broken logger
        # must not restore BaseServer's exception traceback or leak request data.
        pass


class _IPv6HTTPServer(_YapHTTPServer):
    address_family = socket.AF_INET6


class _IPv6ThreadingHTTPServer(_ThreadingYapHTTPServer):
    address_family = socket.AF_INET6


def _server_type(host: str, *, threaded: bool) -> type[HTTPServer]:
    try:
        if ip_address(host).version == 6:
            return _IPv6ThreadingHTTPServer if threaded else _IPv6HTTPServer
    except ValueError:
        pass
    return _ThreadingYapHTTPServer if threaded else _YapHTTPServer


def create_server(
    settings: ServerSettings,
    *,
    logger: logging.Logger | None = None,
    job_service: RecordingJobService | None = None,
) -> HTTPServer:
    ensure_bind_is_allowed(settings.host)
    request_logger = logger or _REQUEST_LOGGER
    handler = partial(
        _HealthRequestHandler,
        request_logger=request_logger,
        job_service=job_service,
    )
    server = _server_type(
        settings.host,
        threaded=job_service is not None,
    )((settings.host, settings.port), handler)
    server._request_error_logger = request_logger
    if isinstance(server, _ThreadingYapHTTPServer):
        server._job_service_for_maintenance = job_service
    return server


def serve(
    settings: ServerSettings,
    *,
    job_service: RecordingJobService | None = None,
) -> None:
    with create_server(settings, job_service=job_service) as server:
        server.serve_forever()
