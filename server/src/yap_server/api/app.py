from __future__ import annotations

import json
import logging
import re
from functools import partial
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any, Mapping
from urllib.parse import urlsplit
from uuid import uuid4

from yap_server.api.health import health
from yap_server.config import ServerSettings
from yap_server.config.settings import ensure_bind_is_allowed
from yap_server.jobs import JobServiceError, RecordingJobService
from .http_server import (
    MAX_CONCURRENT_REQUEST_THREADS,
    ThreadingYapHTTPServer,
    server_type,
)
from .request_io import (
    BoundedRequestBody,
    REQUEST_IO_TIMEOUT_SECONDS,
    bounded_socket_reader,
    request_deadline,
)


MAX_LOG_METHOD_CHARS = 16
MAX_LOG_PATH_CHARS = 128
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
        self._request_body = BoundedRequestBody(self)
        self.requestline = ""
        self.request_version = ""
        self.command = ""
        super().__init__(*args, **kwargs)

    def setup(self) -> None:
        deadline = request_deadline()
        super().setup()
        original_rfile = self.rfile
        self.rfile = bounded_socket_reader(self.connection, deadline)
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
                idempotency_key = self._request_body.required_header(
                    "Idempotency-Key",
                    code="IDEMPOTENCY_KEY_REQUIRED",
                    message="Job creation requires exactly one idempotency key.",
                )
                payload = self._request_body.read_json()
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
                content_length = self._request_body.required_content_length()
                plan = self._job_service.prepare_chunk_upload(
                    chunk_match.group("job_id"),
                    track_id=chunk_match.group("track_id"),
                    sequence_start=int(chunk_match.group("sequence_start"), 10),
                    sequence_end=int(chunk_match.group("sequence_end"), 10),
                    idempotency_key=self._request_body.required_header("Idempotency-Key"),
                    content_sha256=self._request_body.required_header("X-Yap-Content-SHA256"),
                    audio_codec=self._request_body.required_header("X-Yap-Audio-Codec"),
                    sample_rate_hz=self._request_body.integer_header("X-Yap-Sample-Rate-Hz"),
                    channels=self._request_body.integer_header("X-Yap-Channels"),
                    content_length=content_length,
                )
                receipt = self._job_service.accept_chunk(
                    plan,
                    self._request_body.read_exact(content_length),
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
                payload = self._request_body.read_json()
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

    def _request_size_is_allowed(self) -> bool:
        try:
            self._request_body.capture_content_length()
        except JobServiceError as error:
            self._send_error(
                HTTPStatus(error.status),
                code=error.code,
                message=error.message,
                retryable=error.retryable,
            )
            return False
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
        self._request_body.discard_unread()
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
    server = server_type(
        settings.host,
        threaded=job_service is not None,
    )((settings.host, settings.port), handler)
    server._request_error_logger = request_logger
    if isinstance(server, ThreadingYapHTTPServer):
        server._job_service_for_maintenance = job_service
    return server


def serve(
    settings: ServerSettings,
    *,
    job_service: RecordingJobService | None = None,
) -> None:
    with create_server(settings, job_service=job_service) as server:
        server.serve_forever()
